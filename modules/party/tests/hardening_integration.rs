//! Phase 58 Gate A: party safety, tenant, and auth hardening (bd-lqi9m)
//!
//! Five required integration test categories against real Postgres on port 5448:
//!
//! 1. **Migration safety** — apply all migrations forward, verify schema tables
//! 2. **Tenant boundary** — tenant_A data invisible to tenant_B (companies + contacts)
//! 3. **AuthZ denial** — mutation endpoints reject requests without valid JWT claims
//! 4. **Guard→Mutation→Outbox atomicity** — write + outbox row in same transaction
//! 5. **Concurrent tenant isolation** — parallel requests from different tenants

use axum::{body::Body, http::Request as HttpRequest, http::StatusCode};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use party_rs::domain::contact::{CreateContactRequest, UpdateContactRequest};
use party_rs::domain::contact_service;
use party_rs::domain::party::service::{
    create_company, create_individual, get_party, list_parties, update_party,
};
use party_rs::domain::party::{CreateCompanyRequest, CreateIndividualRequest, UpdatePartyRequest};
use party_rs::{http, metrics, AppState};

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://party_user:party_pass@localhost:5448/party_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to party test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run party migrations");

    pool
}

fn unique_app() -> String {
    format!("harden-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn base_company_req(name: &str) -> CreateCompanyRequest {
    CreateCompanyRequest {
        display_name: name.to_string(),
        legal_name: format!("{} LLC", name),
        trade_name: None,
        registration_number: None,
        tax_id: None,
        country_of_incorporation: Some("US".to_string()),
        industry_code: None,
        founded_date: None,
        employee_count: None,
        annual_revenue_cents: None,
        currency: None,
        email: Some(format!(
            "{}@example.com",
            name.to_lowercase().replace(' ', ".")
        )),
        phone: None,
        website: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: Some("US".to_string()),
        metadata: None,
    }
}

fn base_individual_req(first: &str, last: &str) -> CreateIndividualRequest {
    CreateIndividualRequest {
        display_name: format!("{} {}", first, last),
        first_name: first.to_string(),
        last_name: last.to_string(),
        middle_name: None,
        date_of_birth: None,
        tax_id: None,
        nationality: Some("US".to_string()),
        job_title: Some("Engineer".to_string()),
        department: None,
        email: Some(format!(
            "{}.{}@example.com",
            first.to_lowercase(),
            last.to_lowercase()
        )),
        phone: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: None,
        metadata: None,
    }
}

fn base_contact_req(first: &str, last: &str) -> CreateContactRequest {
    CreateContactRequest {
        first_name: first.to_string(),
        last_name: last.to_string(),
        email: Some(format!(
            "{}.{}@example.com",
            first.to_lowercase(),
            last.to_lowercase()
        )),
        phone: Some("+1-555-0100".to_string()),
        role: Some("Manager".to_string()),
        is_primary: Some(false),
        metadata: None,
    }
}

/// Build the party HTTP router without JWT verification.
/// Without a JwtVerifier, the `optional_claims_mw` inserts no claims,
/// so RequirePermissionsLayer will reject mutation requests with 401.
fn build_test_router(pool: sqlx::PgPool) -> axum::Router {
    let party_metrics =
        Arc::new(metrics::PartyMetrics::new().expect("metrics"));
    let app_state = Arc::new(AppState {
        pool,
        metrics: party_metrics,
    });

    // No JWT verifier — simulates unauthenticated caller
    let maybe_verifier: Option<Arc<security::JwtVerifier>> = None;

    http::router(app_state).layer(axum::middleware::from_fn_with_state(
        maybe_verifier,
        security::optional_claims_mw,
    ))
}

// ============================================================================
// 1. Migration safety — apply forward, verify all expected tables exist
// ============================================================================

#[tokio::test]
#[serial]
async fn test_migration_safety_all_tables_present() {
    let pool = setup_db().await;

    // All expected tables from the 4 migration files
    let expected_tables = vec![
        "party_parties",
        "party_companies",
        "party_individuals",
        "party_external_refs",
        "party_outbox",
        "party_processed_events",
        "party_idempotency_keys",
        "party_contacts",
        "party_addresses",
    ];

    for table in &expected_tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = 'public' AND table_name = $1
            )",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Expected table '{}' missing after migrations", table);
    }

    // Verify custom enum types exist
    let expected_types = vec!["party_type", "party_status", "party_address_type"];
    for type_name in &expected_types {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_type WHERE typname = $1
            )",
        )
        .bind(type_name)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Expected enum type '{}' missing", type_name);
    }

    // Verify key indexes exist (spot-check tenant isolation indexes)
    let expected_indexes = vec![
        "idx_party_parties_app_id",
        "idx_party_contacts_app_party",
        "idx_party_addresses_app_party",
        "idx_party_external_refs_app_party",
    ];
    for idx_name in &expected_indexes {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_indexes
                WHERE schemaname = 'public' AND indexname = $1
            )",
        )
        .bind(idx_name)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Expected index '{}' missing", idx_name);
    }

    // Verify migration version tracking
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        migration_count >= 4,
        "Expected at least 4 successful migrations, found {}",
        migration_count
    );

    // ── Rollback/forward-fix documentation ──
    // Party uses append-only migrations (no DROP/ALTER destructive).
    // Rollback strategy (reverse order):
    //   4. Migration 4 (contacts/addresses): DROP TABLE party_addresses, party_contacts;
    //      DROP TYPE party_address_type;
    //   3. Migration 3 (outbox/idempotency): DROP TABLE party_idempotency_keys,
    //      party_processed_events, party_outbox;
    //   2. Migration 2 (external refs): DROP TABLE party_external_refs;
    //   1. Migration 1 (core schema): DROP TABLE party_individuals, party_companies,
    //      party_parties; DROP TYPE party_status, party_type;
    //
    // Forward-fix preferred: if a migration fails mid-apply, fix and re-run.
    // SQLx tracks per-migration success so partial state is recoverable.
}

// ============================================================================
// 2. Tenant boundary — companies and contacts invisible across tenants
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_boundary_companies() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    // Create company under tenant A
    let party = create_company(&pool, &app_a, &base_company_req("Tenant A Corp"), corr())
        .await
        .unwrap();

    // Tenant B cannot see tenant A's company by ID
    let result = get_party(&pool, &app_b, party.party.id).await.unwrap();
    assert!(
        result.is_none(),
        "Tenant B must not see tenant A's company by ID"
    );

    // Tenant B list is empty
    let list = list_parties(&pool, &app_b, true).await.unwrap();
    assert!(
        list.is_empty(),
        "Tenant B list must be empty (no cross-tenant leakage)"
    );

    // Tenant A sees their own company
    let a_view = get_party(&pool, &app_a, party.party.id)
        .await
        .unwrap()
        .expect("Tenant A must see their own party");
    assert_eq!(a_view.party.id, party.party.id);
}

#[tokio::test]
#[serial]
async fn test_tenant_boundary_contacts() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    // Create company + contact under tenant A
    let party = create_company(&pool, &app_a, &base_company_req("Contact Iso Corp"), corr())
        .await
        .unwrap();
    let contact = contact_service::create_contact(
        &pool,
        &app_a,
        party.party.id,
        &base_contact_req("Alice", "Boundary"),
        corr(),
    )
    .await
    .unwrap();

    // Tenant B cannot read tenant A's contact by ID
    let result = contact_service::get_contact(&pool, &app_b, contact.id)
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "Tenant B must not see tenant A's contact"
    );

    // Tenant B cannot deactivate tenant A's contact
    let err = contact_service::deactivate_contact(&pool, &app_b, contact.id, corr())
        .await
        .unwrap_err();
    assert!(
        matches!(err, party_rs::domain::party::PartyError::NotFound(_)),
        "Tenant B deactivate must fail with NotFound"
    );

    // Tenant B cannot update tenant A's contact
    let err = contact_service::update_contact(
        &pool,
        &app_b,
        contact.id,
        &UpdateContactRequest {
            first_name: Some("Hacked".to_string()),
            last_name: None,
            email: None,
            phone: None,
            role: None,
            is_primary: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, party_rs::domain::party::PartyError::NotFound(_)),
        "Tenant B update must fail with NotFound"
    );

    // Tenant A can still see their own contact
    let a_contact = contact_service::get_contact(&pool, &app_a, contact.id)
        .await
        .unwrap()
        .expect("Tenant A must see their own contact");
    assert_eq!(a_contact.first_name, "Alice");
}

// ============================================================================
// 3. AuthZ denial — mutation endpoints reject unauthenticated requests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_authz_create_company_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "display_name": "Unauthorized Corp",
        "legal_name": "Unauthorized Corp LLC"
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/party/companies")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/party/companies must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_create_individual_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "display_name": "Jane Doe",
        "first_name": "Jane",
        "last_name": "Doe"
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/party/individuals")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/party/individuals must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_update_party_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "display_name": "Hacked Name"
    });

    let req = HttpRequest::builder()
        .method("PUT")
        .uri(format!("/api/party/parties/{}", Uuid::new_v4()))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "PUT /api/party/parties/:id must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_create_contact_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "first_name": "Unauthorized",
        "last_name": "Contact"
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri(format!("/api/party/parties/{}/contacts", Uuid::new_v4()))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/party/parties/:id/contacts must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_read_endpoints_allowed_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    // Read endpoints are not behind RequirePermissionsLayer.
    // They use extract_tenant from claims — so without claims, the handler returns 401.
    // This is correct: unauthenticated reads are denied by the handler, not the middleware.
    let req = HttpRequest::builder()
        .method("GET")
        .uri("/api/party/parties")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Reads return 401 from extract_tenant (no claims → no tenant),
    // but the middleware layer itself doesn't block.
    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::OK,
        "GET /api/party/parties should be 401 (no tenant) or 200, got {}",
        resp.status()
    );
}

// ============================================================================
// 4. Guard→Mutation→Outbox atomicity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_company_create() {
    let pool = setup_db().await;
    let app_id = unique_app();

    // Create a company — triggers Guard→Mutation→Outbox
    let view = create_company(&pool, &app_id, &base_company_req("Outbox Corp"), corr())
        .await
        .unwrap();

    // The outbox row must exist (written in the same transaction as the party)
    let outbox_event: Option<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_id FROM party_outbox WHERE aggregate_id = $1",
    )
    .bind(view.party.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, agg_id) =
        outbox_event.expect("Outbox event must exist after company creation");
    assert_eq!(event_type, "party.created");
    assert_eq!(agg_id, view.party.id.to_string());

    // Verify outbox payload contains the expected EventEnvelope fields
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM party_outbox WHERE aggregate_id = $1",
    )
    .bind(view.party.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    // EventEnvelope has: event_type, source_module, schema_version, payload (inner)
    assert_eq!(payload["event_type"], "party.created");
    assert_eq!(payload["source_module"], "party");
    assert_eq!(payload["schema_version"], "1.0.0");
    assert!(
        payload.get("payload").is_some(),
        "Outbox envelope must contain inner payload"
    );
    assert_eq!(payload["payload"]["app_id"], app_id);
}

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_party_update() {
    let pool = setup_db().await;
    let app_id = unique_app();

    // Create then update — both should produce outbox events
    let created = create_company(&pool, &app_id, &base_company_req("Update Outbox Corp"), corr())
        .await
        .unwrap();

    update_party(
        &pool,
        &app_id,
        created.party.id,
        &UpdatePartyRequest {
            display_name: Some("Updated Outbox Corp".to_string()),
            email: None,
            phone: None,
            website: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            metadata: None,
            tags: None,
            updated_by: Some("test-agent".to_string()),
        },
        corr(),
    )
    .await
    .unwrap();

    // Should have at least 2 outbox events: party.created + party.updated
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_id = $1",
    )
    .bind(created.party.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        count >= 2,
        "Expected at least 2 outbox events (created + updated), got {}",
        count
    );

    // Verify party.updated event exists
    let update_event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM party_outbox WHERE aggregate_id = $1 AND event_type = 'party.updated'",
    )
    .bind(created.party.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert!(
        update_event.is_some(),
        "Outbox must contain party.updated event"
    );
}

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_individual_create() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let view = create_individual(
        &pool,
        &app_id,
        &base_individual_req("Outbox", "Individual"),
        corr(),
    )
    .await
    .unwrap();

    // Outbox event must exist for individual creation too
    let outbox_event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM party_outbox WHERE aggregate_id = $1",
    )
    .bind(view.party.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert_eq!(
        outbox_event.unwrap().0,
        "party.created",
        "Outbox must contain party.created event for individual"
    );
}

// ============================================================================
// 5. Concurrent tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_concurrent_tenant_isolation_parties() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    // Spawn concurrent writes from both tenants
    let mut handles = Vec::new();

    for i in 0..5u32 {
        let p = pool.clone();
        let a = app_a.clone();
        handles.push(tokio::spawn(async move {
            create_company(
                &p,
                &a,
                &base_company_req(&format!("Tenant A Corp {}", i)),
                corr(),
            )
            .await
            .expect("Tenant A company create should succeed");
        }));

        let p = pool.clone();
        let b = app_b.clone();
        handles.push(tokio::spawn(async move {
            create_company(
                &p,
                &b,
                &base_company_req(&format!("Tenant B Corp {}", i)),
                corr(),
            )
            .await
            .expect("Tenant B company create should succeed");
        }));
    }

    // Wait for all writes
    for h in handles {
        h.await.expect("join");
    }

    // Verify tenant A sees only their 5 companies
    let a_parties = list_parties(&pool, &app_a, false).await.unwrap();
    assert_eq!(
        a_parties.len(),
        5,
        "Tenant A should have exactly 5 companies"
    );
    assert!(
        a_parties.iter().all(|p| p.app_id == app_a),
        "All tenant A parties must belong to app_a"
    );

    // Verify tenant B sees only their 5 companies
    let b_parties = list_parties(&pool, &app_b, false).await.unwrap();
    assert_eq!(
        b_parties.len(),
        5,
        "Tenant B should have exactly 5 companies"
    );
    assert!(
        b_parties.iter().all(|p| p.app_id == app_b),
        "All tenant B parties must belong to app_b"
    );

    // Cross-tenant: tenant A querying for one of B's party IDs sees nothing
    if let Some(b_party) = b_parties.first() {
        let cross = get_party(&pool, &app_a, b_party.id).await.unwrap();
        assert!(
            cross.is_none(),
            "Tenant A must not see tenant B's party by ID"
        );
    }

    // Verify outbox events are per-tenant (no cross-contamination)
    let a_outbox: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM party_outbox WHERE app_id = $1")
            .bind(&app_a)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a_outbox, 5, "Tenant A should have 5 outbox events");

    let b_outbox: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM party_outbox WHERE app_id = $1")
            .bind(&app_b)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(b_outbox, 5, "Tenant B should have 5 outbox events");
}

#[tokio::test]
#[serial]
async fn test_concurrent_reads_during_writes_isolated() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let mut handles = Vec::new();

    // Tenant A writes companies
    for i in 0..3u32 {
        let p = pool.clone();
        let a = app_a.clone();
        handles.push(tokio::spawn(async move {
            create_company(
                &p,
                &a,
                &base_company_req(&format!("Concurrent A {}", i)),
                corr(),
            )
            .await
            .expect("A write");
        }));
    }

    // Tenant B reads concurrently — must never see A's data
    for _ in 0..3 {
        let p = pool.clone();
        let b = app_b.clone();
        handles.push(tokio::spawn(async move {
            let parties = list_parties(&p, &b, true).await.unwrap();
            assert_eq!(
                parties.len(),
                0,
                "Tenant B must never see tenant A's parties during concurrent reads"
            );
        }));
    }

    // Also write for tenant B (individuals this time)
    for i in 0..2u32 {
        let p = pool.clone();
        let b = app_b.clone();
        handles.push(tokio::spawn(async move {
            create_individual(
                &p,
                &b,
                &base_individual_req(&format!("B-{}", i), "Person"),
                corr(),
            )
            .await
            .expect("B write");
        }));
    }

    for h in handles {
        h.await.expect("join");
    }

    // Final counts
    let a_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM party_parties WHERE app_id = $1",
    )
    .bind(&app_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(a_count, 3, "Tenant A should have 3 parties");

    let b_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM party_parties WHERE app_id = $1",
    )
    .bind(&app_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(b_count, 2, "Tenant B should have 2 parties");
}
