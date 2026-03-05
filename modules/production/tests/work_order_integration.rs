use production_rs::domain::work_orders::{CreateWorkOrderRequest, WorkOrderError, WorkOrderRepo};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to production test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run production migrations");

    pool
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

fn wo_request(tenant: &str, order_num: &str) -> CreateWorkOrderRequest {
    CreateWorkOrderRequest {
        tenant_id: tenant.to_string(),
        order_number: order_num.to_string(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Uuid::new_v4(),
        routing_template_id: None,
        planned_quantity: 10,
        planned_start: None,
        planned_end: None,
        correlation_id: None,
    }
}

// ============================================================================
// Full lifecycle: draft → released → closed
// ============================================================================

#[tokio::test]
#[serial]
async fn work_order_full_lifecycle() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create (draft)
    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-001"), &corr, None)
        .await
        .expect("create");
    assert_eq!(wo.status, "draft");
    assert!(wo.actual_start.is_none());
    assert!(wo.actual_end.is_none());

    // Release
    let released =
        WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
            .await
            .expect("release");
    assert_eq!(released.status, "released");
    assert!(released.actual_start.is_some());
    assert!(released.actual_end.is_none());

    // Close
    let closed =
        WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
            .await
            .expect("close");
    assert_eq!(closed.status, "closed");
    assert!(closed.actual_end.is_some());
}

// ============================================================================
// Events emitted for each transition
// ============================================================================

#[tokio::test]
#[serial]
async fn work_order_events_emitted_for_each_transition() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-EVT"), &corr, None)
        .await
        .expect("create");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release");

    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close");

    let events = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(wo.work_order_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch events");

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "production.work_order_created",
            "production.work_order_released",
            "production.work_order_closed",
        ]
    );
}

// ============================================================================
// Illegal transition: draft → closed rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_close_draft_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DRAFT-CLOSE"), &corr, None)
        .await
        .expect("create");

    let err = WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject draft→closed");

    match err {
        WorkOrderError::InvalidTransition { from, to } => {
            assert_eq!(from, "draft");
            assert_eq!(to, "closed");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Illegal transition: released → released rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_release_already_released_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DBL-REL"), &corr, None)
        .await
        .expect("create");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("first release");

    let err = WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject released→released");

    match err {
        WorkOrderError::InvalidTransition { from, to } => {
            assert_eq!(from, "released");
            assert_eq!(to, "released");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Illegal transition: closed → released rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_release_closed_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-CLOSED-REL"), &corr, None)
        .await
        .expect("create");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release");

    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close");

    let err = WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject closed→released");

    match err {
        WorkOrderError::InvalidTransition { from, to } => {
            assert_eq!(from, "closed");
            assert_eq!(to, "released");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Correlation chain: same correlation_id across all events
// ============================================================================

#[tokio::test]
#[serial]
async fn correlation_id_chains_across_events() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-CORR"), &corr, None)
        .await
        .expect("create");

    // Use same correlation_id for the full lifecycle to simulate a single business flow
    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release");
    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close");

    // Verify all outbox rows for this WO carry the same correlation_id
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT event_type, correlation_id FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(wo.work_order_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch outbox");

    assert_eq!(rows.len(), 3);
    for (event_type, row_corr) in &rows {
        assert_eq!(
            row_corr.as_deref(),
            Some(corr.as_str()),
            "Event {} should carry correlation_id",
            event_type
        );
    }
}

// ============================================================================
// Duplicate correlation_id returns existing WO (idempotency)
// ============================================================================

#[tokio::test]
#[serial]
async fn duplicate_correlation_id_returns_existing_wo() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let dedup_corr = Uuid::new_v4().to_string();

    let mut req = wo_request(&tenant, "WO-IDEM");
    req.correlation_id = Some(dedup_corr.clone());

    let first = WorkOrderRepo::create(&pool, &req, &corr, None)
        .await
        .expect("first create");

    // Second create with same correlation_id: different order number, should return first
    let mut req2 = wo_request(&tenant, "WO-IDEM-2");
    req2.correlation_id = Some(dedup_corr);

    let second = WorkOrderRepo::create(&pool, &req2, &corr, None)
        .await
        .expect("second create should succeed via idempotency");

    assert_eq!(first.work_order_id, second.work_order_id);
    assert_eq!(second.order_number, "WO-IDEM"); // original order number

    // Only one outbox event should exist (from first create)
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.work_order_created'",
    )
    .bind(first.work_order_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count.0, 1, "Duplicate request should not produce extra events");
}

// ============================================================================
// Duplicate order number rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn duplicate_order_number_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DUP"), &corr, None)
        .await
        .expect("first create");

    let err = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DUP"), &corr, None)
        .await
        .expect_err("should reject duplicate order number");

    let msg = format!("{}", err);
    assert!(msg.contains("WO-DUP"), "Error should mention order number: {}", msg);
}

// ============================================================================
// Validation: planned_quantity must be > 0
// ============================================================================

#[tokio::test]
#[serial]
async fn create_rejects_zero_quantity() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let mut req = wo_request(&tenant, "WO-ZERO");
    req.planned_quantity = 0;

    let err = WorkOrderRepo::create(&pool, &req, &corr, None)
        .await
        .expect_err("should reject zero quantity");

    match err {
        WorkOrderError::Validation(msg) => {
            assert!(msg.contains("planned_quantity"), "msg: {}", msg);
        }
        other => panic!("Expected Validation, got: {:?}", other),
    }
}
