//! Integration tests for the crm-pipeline module.
//!
//! Requires a real Postgres database. Set DATABASE_URL or rely on the default.
//! All tests use unique tenant IDs to avoid cross-test interference.

use crm_pipeline_rs::consumers::order_booked::{handle_order_booked, OrderBookedPayload};
use crm_pipeline_rs::domain::contact_role_attributes::{repo as contact_repo, UpsertContactRoleRequest};
use crm_pipeline_rs::domain::leads::{
    service as lead_service, ConvertLeadRequest, CreateLeadRequest,
};
use platform_client_party::{PartiesClient, SearchPartiesQuery};
use platform_sdk::PlatformClient;
use crm_pipeline_rs::domain::opportunities::{
    service as opp_service, AdvanceStageRequest, CloseLostRequest, CloseWonRequest,
    CreateOpportunityRequest, OpportunityError,
};
use crm_pipeline_rs::domain::pipeline_stages::{service as stage_service, CreateStageRequest, StageError};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://crm_pipeline_user:crm_pipeline_pass@localhost:5465/crm_pipeline_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to CRM pipeline test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run CRM pipeline migrations");
    pool
}

fn unique_tenant() -> String {
    format!("crm-test-{}", Uuid::new_v4().simple())
}

fn setup_party_client() -> PartiesClient {
    let url = std::env::var("PARTY_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8098".to_string());
    PartiesClient::new(PlatformClient::new(url))
}

async fn create_qualified_lead(pool: &sqlx::PgPool, tenant_id: &str) -> crm_pipeline_rs::domain::leads::Lead {
    let req = CreateLeadRequest {
        source: "referral".to_string(),
        source_detail: None,
        company_name: "Acme Corp".to_string(),
        contact_name: Some("Jane Smith".to_string()),
        contact_email: None,
        contact_phone: None,
        contact_title: None,
        estimated_value_cents: Some(50_000_00),
        currency: None,
        owner_id: None,
        notes: None,
    };
    let lead = lead_service::create_lead(pool, tenant_id, &req, "test-user".to_string())
        .await
        .expect("create lead");
    let lead = lead_service::mark_contacted(pool, tenant_id, lead.id, "test-user".to_string())
        .await
        .expect("mark contacted");
    let lead = lead_service::mark_qualifying(pool, tenant_id, lead.id, "test-user".to_string())
        .await
        .expect("mark qualifying");
    lead_service::mark_qualified(pool, tenant_id, lead.id, "test-user".to_string())
        .await
        .expect("mark qualified")
}

async fn create_open_opportunity(pool: &sqlx::PgPool, tenant_id: &str) -> crm_pipeline_rs::domain::opportunities::Opportunity {
    stage_service::ensure_default_stages(pool, tenant_id)
        .await
        .expect("seed stages");
    let req = CreateOpportunityRequest {
        title: "Test Opportunity".to_string(),
        party_id: Uuid::new_v4(),
        primary_party_contact_id: None,
        lead_id: None,
        stage_code: None,
        probability_pct: None,
        estimated_value_cents: Some(100_000_00),
        currency: None,
        expected_close_date: None,
        opp_type: None,
        priority: None,
        description: None,
        requirements: None,
        external_quote_ref: None,
        owner_id: None,
    };
    opp_service::create_opportunity(pool, tenant_id, &req, "test-user".to_string())
        .await
        .expect("create opportunity")
}

// ============================================================================
// Lead state machine tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_lead_convert_without_party_id_creates_party() {
    let pool = setup_db().await;
    // Tenant must be a valid UUID so service_claims_from_str can parse it.
    let tenant = Uuid::new_v4().to_string();
    let parties_client = setup_party_client();

    let lead = create_qualified_lead(&pool, &tenant).await;
    assert_eq!(lead.status, "qualified");

    let resp = lead_service::convert_lead(
        &pool,
        &tenant,
        lead.id,
        &ConvertLeadRequest {
            party_id: None,
            party_contact_id: None,
            opportunity_title: None,
        },
        Some(&parties_client),
    )
    .await
    .expect("auto-create party + convert should succeed");

    assert_eq!(resp.lead.status, "converted");
    let new_party_id = resp.lead.party_id.expect("party_id must be set after auto-create");

    // Verify the Party company exists in the Party service.
    let tenant_uuid = Uuid::parse_str(&tenant).expect("tenant is a valid UUID");
    let service_claims = PlatformClient::service_claims(tenant_uuid);
    let search = parties_client
        .search_parties(
            &service_claims,
            &SearchPartiesQuery {
                name: Some(lead.company_name.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("search parties");

    assert!(!search.data.is_empty(), "Party company must exist after auto-create");
    assert_eq!(
        search.data[0].id, new_party_id,
        "returned party_id must match the created Party row"
    );

    // A second conversion with the same company_name either reuses via Party's 409
    // or creates a second row — pin whichever behaviour Party gives us.
    let lead2 = create_qualified_lead(&pool, &tenant).await;
    let result2 = lead_service::convert_lead(
        &pool,
        &tenant,
        lead2.id,
        &ConvertLeadRequest {
            party_id: None,
            party_contact_id: None,
            opportunity_title: None,
        },
        Some(&parties_client),
    )
    .await;
    match result2 {
        Ok(resp2) => assert_eq!(resp2.lead.status, "converted"),
        Err(e) => {
            let s = format!("{e:?}");
            assert!(
                s.contains("409") || s.contains("party_api"),
                "unexpected error on second conversion: {s}"
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn test_lead_convert_with_party_id_succeeds() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let lead = create_qualified_lead(&pool, &tenant).await;
    let party_id = Uuid::new_v4();

    let result = lead_service::convert_lead(
        &pool,
        &tenant,
        lead.id,
        &ConvertLeadRequest {
            party_id: Some(party_id),
            party_contact_id: None,
            opportunity_title: None,
        },
        None,
    )
    .await
    .expect("convert lead");

    assert_eq!(result.lead.status, "converted");
    assert_eq!(result.lead.party_id, Some(party_id));
}

// ============================================================================
// Pipeline stage tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_default_stages_seeded_idempotent() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    stage_service::ensure_default_stages(&pool, &tenant)
        .await
        .expect("first seed");
    stage_service::ensure_default_stages(&pool, &tenant)
        .await
        .expect("second seed (idempotent)");

    let stages = stage_service::list_stages(&pool, &tenant)
        .await
        .expect("list stages");

    assert_eq!(
        stages.len(),
        7,
        "Expected 7 default stages, got {}",
        stages.len()
    );
    assert!(stages.iter().any(|s| s.stage_code == "prospecting" && !s.is_terminal));
    assert!(stages.iter().any(|s| s.stage_code == "closed_won" && s.is_terminal && s.is_win));
    assert!(stages.iter().any(|s| s.stage_code == "closed_lost" && s.is_terminal && !s.is_win));
}

#[tokio::test]
#[serial]
async fn test_duplicate_order_rank_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    // Create first non-terminal stage at order_rank 100
    stage_service::create_stage(
        &pool,
        &tenant,
        &CreateStageRequest {
            stage_code: "alpha".to_string(),
            display_label: "Alpha".to_string(),
            description: None,
            order_rank: 100,
            is_terminal: false,
            is_win: None,
            probability_default_pct: None,
        },
        "test-user",
    )
    .await
    .expect("create alpha stage");

    // Second non-terminal stage at same order_rank → MultipleInitialStages
    let result = stage_service::create_stage(
        &pool,
        &tenant,
        &CreateStageRequest {
            stage_code: "beta".to_string(),
            display_label: "Beta".to_string(),
            description: None,
            order_rank: 100,
            is_terminal: false,
            is_win: None,
            probability_default_pct: None,
        },
        "test-user",
    )
    .await;

    assert!(
        matches!(result, Err(StageError::MultipleInitialStages)),
        "Expected MultipleInitialStages, got {:?}",
        result
    );
}

#[tokio::test]
#[serial]
async fn test_tenant_isolation_stages() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    stage_service::ensure_default_stages(&pool, &tenant_a)
        .await
        .expect("seed tenant A");

    let stages_b = stage_service::list_stages(&pool, &tenant_b)
        .await
        .expect("list tenant B");

    assert!(stages_b.is_empty(), "Tenant B must not see Tenant A's stages");
}

// ============================================================================
// Opportunity tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_advance_stage_to_terminal_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let opp = create_open_opportunity(&pool, &tenant).await;

    let result = opp_service::advance_stage(
        &pool,
        &tenant,
        opp.id,
        &AdvanceStageRequest {
            stage_code: "closed_won".to_string(),
            probability_pct: None,
            reason: None,
            notes: None,
        },
        "test-user".to_string(),
    )
    .await;

    assert!(
        matches!(result, Err(OpportunityError::TerminalStageViaAdvance(_))),
        "Expected TerminalStageViaAdvance, got {:?}",
        result
    );
}

#[tokio::test]
#[serial]
async fn test_stage_history_is_append_only() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let opp = create_open_opportunity(&pool, &tenant).await;

    let initial_detail = opp_service::get_opportunity_detail(&pool, &tenant, opp.id)
        .await
        .expect("get detail");
    assert_eq!(initial_detail.stage_history.len(), 1, "Initial history entry on creation");

    // Advance to discovery
    opp_service::advance_stage(
        &pool,
        &tenant,
        opp.id,
        &AdvanceStageRequest {
            stage_code: "discovery".to_string(),
            probability_pct: None,
            reason: None,
            notes: None,
        },
        "test-user".to_string(),
    )
    .await
    .expect("advance to discovery");

    let detail = opp_service::get_opportunity_detail(&pool, &tenant, opp.id)
        .await
        .expect("get detail after advance");
    assert_eq!(detail.stage_history.len(), 2, "History must grow after advance");

    // Advance to proposal
    opp_service::advance_stage(
        &pool,
        &tenant,
        opp.id,
        &AdvanceStageRequest {
            stage_code: "proposal".to_string(),
            probability_pct: None,
            reason: None,
            notes: None,
        },
        "test-user".to_string(),
    )
    .await
    .expect("advance to proposal");

    let detail2 = opp_service::get_opportunity_detail(&pool, &tenant, opp.id)
        .await
        .expect("get detail after second advance");
    assert_eq!(detail2.stage_history.len(), 3, "History must grow again");
}

#[tokio::test]
#[serial]
async fn test_close_won_sets_actual_close_date() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let opp = create_open_opportunity(&pool, &tenant).await;

    let won = opp_service::close_won(
        &pool,
        &tenant,
        opp.id,
        &CloseWonRequest {
            sales_order_id: None,
            reason: Some("Deal closed".to_string()),
            notes: None,
        },
        "test-user".to_string(),
    )
    .await
    .expect("close won");

    assert!(won.actual_close_date.is_some(), "actual_close_date must be set on close-won");
    assert_eq!(won.stage_code, "closed_won");
    assert_eq!(won.probability_pct, 100);
}

#[tokio::test]
#[serial]
async fn test_close_lost_requires_reason() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let opp = create_open_opportunity(&pool, &tenant).await;

    let result = opp_service::close_lost(
        &pool,
        &tenant,
        opp.id,
        &CloseLostRequest {
            close_reason: "".to_string(),
            competitor: None,
            notes: None,
        },
        "test-user".to_string(),
    )
    .await;

    assert!(
        matches!(result, Err(OpportunityError::CloseLostRequiresReason)),
        "Expected CloseLostRequiresReason, got {:?}",
        result
    );
}

// ============================================================================
// Consumer side-effect tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_contact_deactivated_sets_inactive() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let contact_id = Uuid::new_v4();

    // Upsert contact role attributes first
    contact_repo::upsert_attributes(
        &pool,
        &tenant,
        contact_id,
        &UpsertContactRoleRequest {
            sales_role: Some("champion".to_string()),
            is_primary_buyer: Some(true),
            is_economic_buyer: None,
            notes: None,
        },
        "test-user",
    )
    .await
    .expect("upsert contact role");

    // Verify is_active starts true
    let attrs = contact_repo::get_attributes(&pool, &tenant, contact_id)
        .await
        .expect("get attrs")
        .expect("attrs must exist");
    assert!(attrs.is_active);

    // Simulate party.contact.deactivated handler
    contact_repo::deactivate_contact(&pool, &tenant, contact_id)
        .await
        .expect("deactivate contact");

    let attrs_after = contact_repo::get_attributes(&pool, &tenant, contact_id)
        .await
        .expect("get attrs after")
        .expect("attrs must still exist");
    assert!(!attrs_after.is_active, "is_active must be false after deactivation");
}

#[tokio::test]
#[serial]
async fn test_order_booked_links_sales_order_to_opportunity() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let opp = create_open_opportunity(&pool, &tenant).await;

    let sales_order_id = Uuid::new_v4();
    let payload = OrderBookedPayload {
        sales_order_id,
        tenant_id: tenant.clone(),
        opportunity_id: Some(opp.id),
    };

    handle_order_booked(&pool, &payload).await;

    let opp_after = opp_service::get_opportunity(&pool, &tenant, opp.id)
        .await
        .expect("get opportunity after order booked");
    assert_eq!(
        opp_after.sales_order_id,
        Some(sales_order_id),
        "sales_order_id must be set after order.booked event"
    );
}
