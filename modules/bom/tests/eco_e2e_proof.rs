/// ECO lifecycle end-to-end proof — Phase D integration evidence.
///
/// Ties together: auto-numbering → ECO lifecycle → BOM revision supersession
/// → date-based effectivity → history queries → outbox event chain.
///
/// This is NOT a retest of individual functions (those are in eco_integration.rs
/// and eco_numbering_integration.rs). This proves the CHAIN works as a unit.
use bom_rs::domain::bom_service;
use bom_rs::domain::eco_models::*;
use bom_rs::domain::eco_service;
use bom_rs::domain::models::*;
use bom_rs::domain::numbering_client::NumberingClient;
use chrono::{Duration, Utc};
use platform_sdk::PlatformClient;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_bom_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://bom_user:bom_pass@localhost:5450/bom_db?sslmode=require".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run BOM migrations");

    pool
}

async fn setup_numbering_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("NUMBERING_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://numbering_user:numbering_pass@localhost:5456/numbering_db".to_string()
    });

    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to Numbering test DB")
}

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

async fn setup_bom_with_revisions(
    pool: &sqlx::PgPool,
    tenant: &str,
) -> (BomHeader, BomRevision, BomRevision) {
    let corr = Uuid::new_v4().to_string();
    let part_id = Uuid::new_v4();

    let header = bom_service::create_bom(
        pool,
        tenant,
        &CreateBomRequest {
            part_id,
            description: Some("ECO E2E Proof Assembly".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    // Rev-A: effective (30 days ago)
    let rev_a = bom_service::create_revision(
        pool,
        tenant,
        header.id,
        &CreateRevisionRequest {
            revision_label: "Rev-A".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create rev-a");

    bom_service::add_line(
        pool,
        tenant,
        rev_a.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 2.0,
            uom: Some("EA".to_string()),
            scrap_factor: None,
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add line to rev-a");

    let rev_a = bom_service::set_effectivity(
        pool,
        tenant,
        rev_a.id,
        &SetEffectivityRequest {
            effective_from: Utc::now() - Duration::days(30),
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .expect("set effectivity rev-a");

    assert_eq!(rev_a.status, "effective");

    // Rev-B: draft (the new revision ECO will release)
    let rev_b = bom_service::create_revision(
        pool,
        tenant,
        header.id,
        &CreateRevisionRequest {
            revision_label: "Rev-B".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create rev-b");

    bom_service::add_line(
        pool,
        tenant,
        rev_b.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 3.0,
            uom: Some("EA".to_string()),
            scrap_factor: Some(0.02),
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add line to rev-b");

    (header, rev_a, rev_b)
}

// ============================================================================
// E2E: Full ECO lifecycle with auto-numbering → BOM supersession → queries
// ============================================================================

#[tokio::test]
#[serial]
async fn e2e_eco_full_lifecycle_with_numbering() {
    let bom_pool = setup_bom_db().await;
    let num_pool = setup_numbering_db().await;
    let tenant = unique_tenant();
    let numbering = NumberingClient::direct(num_pool);
    let corr = Uuid::new_v4().to_string();

    let (header, rev_a, rev_b) = setup_bom_with_revisions(&bom_pool, &tenant).await;

    // ---- Step 1: Create ECO with auto-allocated number ----
    let eco = eco_service::create_eco(
        &bom_pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: None, // auto-allocate from Numbering service
            title: "Replace legacy component with updated spec".to_string(),
            description: Some("Full lifecycle proof".to_string()),
            created_by: "engineer-1".to_string(),
        },
        Some(&numbering),
        None,
        &corr,
        None,
        &PlatformClient::service_claims(Uuid::new_v4()),
    )
    .await
    .expect("create eco with auto-numbering");

    assert_eq!(eco.status, "draft");
    assert_eq!(
        eco.eco_number, "ECO-00001",
        "First ECO should get ECO-00001"
    );

    // ---- Step 2: Link BOM revisions (Rev-A → Rev-B) ----
    let bom_link = eco_service::link_bom_revision(
        &bom_pool,
        &tenant,
        eco.id,
        &LinkBomRevisionRequest {
            bom_id: header.id,
            before_revision_id: rev_a.id,
            after_revision_id: rev_b.id,
        },
    )
    .await
    .expect("link bom revision");

    assert_eq!(bom_link.before_revision_id, rev_a.id);
    assert_eq!(bom_link.after_revision_id, rev_b.id);

    // ---- Step 3: Link doc revision (evidence) ----
    let doc_id = Uuid::new_v4();
    let doc_rev_id = Uuid::new_v4();
    eco_service::link_doc_revision(
        &bom_pool,
        &tenant,
        eco.id,
        &LinkDocRevisionRequest {
            doc_id,
            doc_revision_id: doc_rev_id,
        },
    )
    .await
    .expect("link doc revision");

    // ---- Step 4: Submit ----
    let eco = eco_service::submit_eco(
        &bom_pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "engineer-1".to_string(),
            comment: Some("Ready for review".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("submit eco");

    assert_eq!(eco.status, "submitted");

    // ---- Step 5: Approve ----
    let eco = eco_service::approve_eco(
        &bom_pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "manager-1".to_string(),
            comment: Some("Approved for implementation".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("approve eco");

    assert_eq!(eco.status, "approved");
    assert!(eco.approved_by.is_some());

    // ---- Step 6: Apply → BOM revision supersession ----
    let effective_from = Utc::now();
    let eco = eco_service::apply_eco(
        &bom_pool,
        &tenant,
        eco.id,
        &ApplyEcoRequest {
            actor: "engineer-1".to_string(),
            effective_from,
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .expect("apply eco");

    assert_eq!(eco.status, "applied");
    assert!(eco.applied_at.is_some());

    // ---- Step 7: Verify BOM revision states ----
    let rev_a_after = bom_service::get_revision(&bom_pool, &tenant, rev_a.id)
        .await
        .expect("get rev-a after apply");
    assert_eq!(
        rev_a_after.status, "superseded",
        "Rev-A must be superseded after ECO apply"
    );

    let rev_b_after = bom_service::get_revision(&bom_pool, &tenant, rev_b.id)
        .await
        .expect("get rev-b after apply");
    assert_eq!(
        rev_b_after.status, "effective",
        "Rev-B must be effective after ECO apply"
    );
    assert!(
        rev_b_after.effective_from.is_some(),
        "Rev-B must have effectivity date set"
    );

    // ---- Step 8: Query "ECO history for part" ----
    let history = eco_service::eco_history_for_part(&bom_pool, &tenant, header.part_id)
        .await
        .expect("eco history for part");

    assert_eq!(
        history.len(),
        1,
        "Should have exactly one ECO for this part"
    );
    assert_eq!(
        history[0].eco_number, "ECO-00001",
        "History should show auto-allocated number"
    );
    assert_eq!(history[0].status, "applied");

    // ---- Step 9: Query "BOM rev effective on date X" ----
    let explosion = bom_service::explode(
        &bom_pool,
        &tenant,
        header.id,
        &ExplosionQuery {
            date: Some(effective_from + Duration::seconds(1)),
            max_depth: None,
        },
    )
    .await
    .expect("explosion at post-ECO date");

    assert!(
        !explosion.is_empty(),
        "Explosion should return rows at post-ECO date"
    );
    assert_eq!(
        explosion[0].revision_label, "Rev-B",
        "Post-ECO date should resolve to Rev-B (not superseded Rev-A)"
    );

    // ---- Step 10: Verify doc revision link is queryable ----
    let doc_links = eco_service::list_doc_revision_links(&bom_pool, &tenant, eco.id)
        .await
        .expect("doc links");
    assert_eq!(doc_links.len(), 1);
    assert_eq!(doc_links[0].doc_revision_id, doc_rev_id);

    // ---- Step 11: Verify full audit trail ----
    let audit = eco_service::get_audit_trail(&bom_pool, &tenant, eco.id)
        .await
        .expect("audit trail");

    let actions: Vec<&str> = audit.iter().map(|a| a.action.as_str()).collect();
    assert_eq!(
        actions,
        vec!["created", "submitted", "approved", "applied"],
        "Audit trail must show complete lifecycle"
    );

    // ---- Step 12: Verify outbox events ----
    let eco_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM bom_outbox WHERE tenant_id = $1 AND event_type LIKE 'eco.%' ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&bom_pool)
    .await
    .expect("eco outbox events");

    let eco_types: Vec<&str> = eco_events.iter().map(|r| r.0.as_str()).collect();
    assert!(
        eco_types.contains(&"eco.created"),
        "Missing eco.created event"
    );
    assert!(
        eco_types.contains(&"eco.submitted"),
        "Missing eco.submitted event"
    );
    assert!(
        eco_types.contains(&"eco.approved"),
        "Missing eco.approved event"
    );
    assert!(
        eco_types.contains(&"eco.applied"),
        "Missing eco.applied event"
    );

    let rev_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM bom_outbox WHERE tenant_id = $1 AND event_type LIKE 'bom.revision_%' ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&bom_pool)
    .await
    .expect("revision outbox events");

    let rev_types: Vec<&str> = rev_events.iter().map(|r| r.0.as_str()).collect();
    assert!(
        rev_types.contains(&"bom.revision_superseded"),
        "Missing bom.revision_superseded event"
    );
    assert!(
        rev_types.contains(&"bom.revision_released"),
        "Missing bom.revision_released event"
    );
}

// ============================================================================
// E2E: Sequential numbering across multiple ECOs
// ============================================================================

#[tokio::test]
#[serial]
async fn e2e_eco_sequential_numbering() {
    let bom_pool = setup_bom_db().await;
    let num_pool = setup_numbering_db().await;
    let tenant = unique_tenant();
    let numbering = NumberingClient::direct(num_pool);

    let mut eco_numbers = Vec::new();

    for i in 1..=3 {
        let eco = eco_service::create_eco(
            &bom_pool,
            &tenant,
            &CreateEcoRequest {
                eco_number: None,
                title: format!("Sequential ECO #{}", i),
                description: None,
                created_by: "engineer-1".to_string(),
            },
            Some(&numbering),
            None,
            &Uuid::new_v4().to_string(),
            None,
            &PlatformClient::service_claims(Uuid::new_v4()),
        )
        .await
        .unwrap_or_else(|e| panic!("create eco #{}: {}", i, e));

        eco_numbers.push(eco.eco_number);
    }

    assert_eq!(eco_numbers[0], "ECO-00001", "First ECO number");
    assert_eq!(eco_numbers[1], "ECO-00002", "Second ECO number");
    assert_eq!(eco_numbers[2], "ECO-00003", "Third ECO number");

    // Verify uniqueness
    let mut unique = eco_numbers.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "All 3 ECO numbers must be unique");
}
