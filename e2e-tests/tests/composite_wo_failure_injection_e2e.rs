// E2E: Composite WO failure injection tests (bd-xiz0k — GAP-22)
//
// Verifies the idempotency and recovery invariants for the composite WO
// create endpoint.  All tests use the NumberingClient::direct and
// WorkOrderRepo::composite_create directly against real Postgres databases —
// no mocks, no stubs.
//
// Invariant under test:
//   A caller who retries composite_create with the same idempotency_key MUST
//   always receive the same WO number (or the existing WO) — never a new WO
//   with a new number.
//
// Test databases:
//   Production: PRODUCTION_DATABASE_URL (default: localhost:5461)
//   Numbering:  NUMBERING_DATABASE_URL  (default: localhost:5456)

use production_rs::domain::bom_client::BomRevisionClient;
use production_rs::domain::numbering_client::NumberingClient;
use production_rs::domain::work_orders::{CompositeCreateWorkOrderRequest, WorkOrderRepo};
use security::{ActorType, VerifiedClaims};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── Database setup ─────────────────────────────────────────────────────────────

async fn setup_production_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("PRODUCTION_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("connect to production DB");
    sqlx::migrate!("../modules/production/db/migrations")
        .run(&pool)
        .await
        .expect("run production migrations");
    pool
}

async fn setup_numbering_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("NUMBERING_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://numbering_user:numbering_pass@localhost:5456/numbering_db".to_string()
    });
    PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("connect to numbering DB")
}

fn make_test_claims(tenant_id: &str) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::parse_str(tenant_id).unwrap_or_else(|_| Uuid::new_v4()),
        app_id: None,
        roles: vec!["admin".to_string()],
        perms: vec!["production.mutate".to_string(), "production.read".to_string()],
        actor_type: ActorType::User,
        issued_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// Test 1 — No orphaned WO number after simulated INSERT failure.
///
/// Scenario: Numbering allocates WO-NNNNN.  The caller's INSERT fails
/// (simulated here by pre-allocating the number via the direct client and
/// NOT creating the WO).  Retrying with the SAME idempotency_key MUST:
///   a) receive the same WO number from Numbering (idempotency preserved), AND
///   b) successfully create the WO with that number.
///
/// Passes if retries are convergent — the final state is exactly one WO
/// bearing the originally-allocated number.
#[tokio::test]
async fn retry_after_insert_failure_gets_same_number() {
    let prod_pool = setup_production_db().await;
    let num_pool = setup_numbering_db().await;
    let numbering = NumberingClient::direct(num_pool);

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let idem_key = format!("failure-inject:retry:{}", tenant_uuid);

    // Step 1: Pre-allocate the WO number (simulates the successful Numbering
    // call in composite_create before the INSERT).
    let pre_allocated = numbering
        .allocate_wo_number(&tenant, &idem_key, &claims)
        .await
        .expect("pre-allocate WO number");

    assert!(
        pre_allocated.starts_with("WO-"),
        "expected WO-NNNNN format, got: {}",
        pre_allocated
    );

    // Step 2: Simulate INSERT failure — do NOT create the WO.
    // The production DB has no WO for this tenant yet.
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM work_orders WHERE tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&prod_pool)
    .await
    .expect("count query");
    assert_eq!(count, 0, "no WO should exist yet");

    // Step 3: Retry composite_create with the SAME idempotency_key.
    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: None,
        routing_template_id: None,
        planned_quantity: 5,
        planned_start: None,
        planned_end: None,
        idempotency_key: idem_key.clone(),
    };
    let corr = Uuid::new_v4().to_string();
    let bom = BomRevisionClient::permissive();
    let wo = WorkOrderRepo::composite_create(&prod_pool, &numbering, &bom, &req, &claims, &corr, None)
        .await
        .expect("composite_create retry must succeed");

    // The retry must receive the SAME WO number that was pre-allocated.
    assert_eq!(
        wo.order_number, pre_allocated,
        "retry with same idempotency_key must return same WO number"
    );
    assert_eq!(wo.status, "draft");
    assert_eq!(wo.tenant_id, tenant);
}

/// Test 2 — Idempotent retry returns the same WO.
///
/// Scenario: composite_create succeeds on the first call.  A second call
/// with the SAME idempotency_key must return the same WO (same order_number,
/// same work_order_id) without creating a duplicate.
#[tokio::test]
async fn idempotent_retry_returns_same_wo() {
    let prod_pool = setup_production_db().await;
    let num_pool = setup_numbering_db().await;
    let numbering = NumberingClient::direct(num_pool);

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let idem_key = format!("idempotent-retry:{}", tenant_uuid);

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: None,
        routing_template_id: None,
        planned_quantity: 3,
        planned_start: None,
        planned_end: None,
        idempotency_key: idem_key.clone(),
    };

    let bom = BomRevisionClient::permissive();
    // First call — creates WO.
    let corr1 = Uuid::new_v4().to_string();
    let wo1 = WorkOrderRepo::composite_create(&prod_pool, &numbering, &bom, &req, &claims, &corr1, None)
        .await
        .expect("first composite_create");

    // Second call — same idempotency_key, same tenant, same item.
    let corr2 = Uuid::new_v4().to_string();
    let wo2 = WorkOrderRepo::composite_create(&prod_pool, &numbering, &bom, &req, &claims, &corr2, None)
        .await
        .expect("second composite_create (idempotent)");

    assert_eq!(
        wo1.order_number, wo2.order_number,
        "same idempotency_key → same WO number on retry"
    );
    // Both calls must refer to the same WO (same ID or same number for the tenant).
    assert_eq!(
        wo1.work_order_id, wo2.work_order_id,
        "same idempotency_key → same WO returned"
    );
}

/// Test 3 — Concurrent requests with the same idempotency_key produce exactly one WO.
///
/// Scenario: two goroutines fire composite_create simultaneously with the
/// same idempotency_key.  The expected outcome is exactly one WO in the
/// production database bearing the allocated number.
#[tokio::test]
async fn concurrent_same_key_creates_exactly_one_wo() {
    let prod_pool = setup_production_db().await;
    let num_pool = setup_numbering_db().await;

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let idem_key = format!("concurrent:{}", tenant_uuid);
    let item_id = Uuid::new_v4();

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id,
        bom_revision_id: None,
        routing_template_id: None,
        planned_quantity: 2,
        planned_start: None,
        planned_end: None,
        idempotency_key: idem_key.clone(),
    };

    // Two separate clients sharing the same pool (PgPool is Arc-backed and cheap to clone).
    let numbering_a = NumberingClient::direct(num_pool.clone());
    let numbering_b = NumberingClient::direct(num_pool);
    let corr1 = Uuid::new_v4().to_string();
    let corr2 = Uuid::new_v4().to_string();
    let bom = BomRevisionClient::permissive();

    let (r1, r2) = tokio::join!(
        WorkOrderRepo::composite_create(&prod_pool, &numbering_a, &bom, &req, &claims, &corr1, None),
        WorkOrderRepo::composite_create(&prod_pool, &numbering_b, &bom, &req, &claims, &corr2, None),
    );

    // Both must succeed: one creates the WO, the other returns the existing one.
    let wo1 = r1.expect("first concurrent composite_create");
    let wo2 = r2.expect("second concurrent composite_create");

    // Same idempotency_key in Numbering → same order_number.
    assert_eq!(
        wo1.order_number, wo2.order_number,
        "concurrent requests with same key get same WO number"
    );

    // Exactly one WO row in DB (no duplicates created).
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM work_orders WHERE tenant_id = $1 AND order_number = $2",
    )
    .bind(&tenant)
    .bind(&wo1.order_number)
    .fetch_one(&prod_pool)
    .await
    .expect("count WOs");
    assert_eq!(count, 1, "exactly one WO must exist for the allocated number");
}
