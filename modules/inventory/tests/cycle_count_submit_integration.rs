//! Integration tests for cycle count submit mutation (bd-1q0j).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Submit open task with counted quantities → status = submitted, variance computed
//! 2. Idempotent submit (same key, same body) → 200 OK, same result returned
//! 3. Conflicting idempotency key (different body) → error
//! 4. Submit non-open task (already submitted) → TaskNotOpen error
//! 5. Submit task that doesn't exist → TaskNotFound error
//! 6. Submit task from wrong tenant → TaskNotFound error
//! 7. Submit with unknown line_id → LineNotFound error
//! 8. Submit with negative counted_qty → NegativeCountedQty error
//! 9. Outbox event emitted with correct payload
//! 10. Submit empty lines (task with zero lines) → succeeds, zero lines in result

use inventory_rs::domain::{
    cycle_count::{
        submit_service::{submit_cycle_count, SubmitError, SubmitLineInput, SubmitRequest},
        task_service::{create_cycle_count_task, CreateTaskRequest, TaskScope},
    },
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    locations::{CreateLocationRequest, LocationRepo},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for integration tests");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");
    pool
}

fn unique_tenant() -> String {
    format!("cs-tenant-{}", Uuid::new_v4())
}

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Submit Test Item {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    }
}

fn make_location_req(tenant_id: &str, warehouse_id: Uuid, code: &str) -> CreateLocationRequest {
    CreateLocationRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        code: code.to_string(),
        name: format!("Submit Test Location {}", code),
        description: None,
    }
}

fn receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    qty: i64,
    key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: Some(location_id),
        quantity: qty,
        unit_cost_minor: 1000,
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

/// Create a task with one item and some on-hand stock. Returns (task_id, line_id, item_id).
async fn setup_task_with_stock(
    pool: &sqlx::PgPool,
    tenant: &str,
    warehouse_id: Uuid,
    sku_suffix: &str,
    loc_code: &str,
    on_hand_qty: i64,
) -> (Uuid, Uuid, Uuid) {
    let item = ItemRepo::create(pool, &make_item_req(tenant, &format!("CS-{}", sku_suffix)))
        .await
        .expect("create item");

    let loc = LocationRepo::create(pool, &make_location_req(tenant, warehouse_id, loc_code))
        .await
        .expect("create location");

    process_receipt(pool, &receipt_req(tenant, item.id, warehouse_id, loc.id, on_hand_qty, &format!("cs-rcpt-{}", sku_suffix)))
        .await
        .expect("receipt");

    let task_req = CreateTaskRequest {
        tenant_id: tenant.to_string(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Full,
        item_ids: vec![],
    };
    let task = create_cycle_count_task(pool, &task_req)
        .await
        .expect("create task");

    assert_eq!(task.lines.len(), 1);
    let line_id = task.lines[0].line_id;
    (task.task_id, line_id, item.id)
}

fn submit_req(task_id: Uuid, tenant_id: &str, key: &str, lines: Vec<SubmitLineInput>) -> SubmitRequest {
    SubmitRequest {
        task_id,
        tenant_id: tenant_id.to_string(),
        idempotency_key: key.to_string(),
        lines,
        correlation_id: None,
        causation_id: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

/// 1. Submit open task with counted quantities → status = submitted, variance computed.
#[tokio::test]
#[serial]
async fn test_submit_open_task_computes_variance() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, line_id, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB01", "S-01", 50).await;

    let req = submit_req(task_id, &tenant, "submit-01", vec![
        SubmitLineInput { line_id, counted_qty: 45 },
    ]);
    let (result, is_replay) = submit_cycle_count(&pool, &req).await.expect("submit");

    assert!(!is_replay);
    assert_eq!(result.status, "submitted");
    assert_eq!(result.task_id, task_id);
    assert_eq!(result.line_count, 1);
    assert_eq!(result.lines[0].line_id, line_id);
    assert_eq!(result.lines[0].expected_qty, 50);
    assert_eq!(result.lines[0].counted_qty, 45);
    assert_eq!(result.lines[0].variance_qty, -5);
}

/// 2. Idempotent submit (same key, same body) → is_replay = true, same result.
#[tokio::test]
#[serial]
async fn test_submit_idempotent_replay() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, line_id, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB02", "S-02", 30).await;

    let req = submit_req(task_id, &tenant, "submit-02", vec![
        SubmitLineInput { line_id, counted_qty: 28 },
    ]);

    let (result1, replay1) = submit_cycle_count(&pool, &req).await.expect("first submit");
    assert!(!replay1);

    let (result2, replay2) = submit_cycle_count(&pool, &req).await.expect("second submit");
    assert!(replay2);
    assert_eq!(result1.task_id, result2.task_id);
    assert_eq!(result1.lines[0].counted_qty, result2.lines[0].counted_qty);
    assert_eq!(result1.lines[0].variance_qty, result2.lines[0].variance_qty);
}

/// 3. Conflicting idempotency key (same key, different body) → error.
#[tokio::test]
#[serial]
async fn test_submit_conflicting_idempotency_key() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, line_id, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB03", "S-03", 20).await;

    let req1 = submit_req(task_id, &tenant, "submit-03-conflict", vec![
        SubmitLineInput { line_id, counted_qty: 18 },
    ]);
    submit_cycle_count(&pool, &req1).await.expect("first submit");

    // Same key, different counted_qty
    let req2 = submit_req(task_id, &tenant, "submit-03-conflict", vec![
        SubmitLineInput { line_id, counted_qty: 19 },
    ]);
    let err = submit_cycle_count(&pool, &req2).await.expect_err("should conflict");
    assert!(matches!(err, SubmitError::ConflictingIdempotencyKey));
}

/// 4. Submit non-open task (already submitted) → TaskNotOpen error.
#[tokio::test]
#[serial]
async fn test_submit_already_submitted_task() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, line_id, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB04", "S-04", 10).await;

    // First submit
    submit_cycle_count(&pool, &submit_req(task_id, &tenant, "submit-04a", vec![
        SubmitLineInput { line_id, counted_qty: 10 },
    ])).await.expect("first submit");

    // Second submit with different key (task now submitted)
    let err = submit_cycle_count(&pool, &submit_req(task_id, &tenant, "submit-04b", vec![
        SubmitLineInput { line_id, counted_qty: 10 },
    ])).await.expect_err("should fail");

    assert!(matches!(err, SubmitError::TaskNotOpen { .. }));
}

/// 5. Submit task that doesn't exist → TaskNotFound error.
#[tokio::test]
#[serial]
async fn test_submit_nonexistent_task() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let err = submit_cycle_count(&pool, &submit_req(
        Uuid::new_v4(), &tenant, "submit-05", vec![],
    )).await.expect_err("should fail");

    assert!(matches!(err, SubmitError::TaskNotFound));
}

/// 6. Submit task belonging to a different tenant → TaskNotFound error.
#[tokio::test]
#[serial]
async fn test_submit_wrong_tenant() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, _, _) = setup_task_with_stock(&pool, &tenant_a, wh, "SUB06", "S-06", 15).await;

    let err = submit_cycle_count(&pool, &submit_req(
        task_id, &tenant_b, "submit-06", vec![],
    )).await.expect_err("should fail");

    assert!(matches!(err, SubmitError::TaskNotFound));
}

/// 7. Submit with unknown line_id → LineNotFound error.
#[tokio::test]
#[serial]
async fn test_submit_unknown_line_id() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, _, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB07", "S-07", 25).await;

    let err = submit_cycle_count(&pool, &submit_req(
        task_id, &tenant, "submit-07",
        vec![SubmitLineInput { line_id: Uuid::new_v4(), counted_qty: 25 }],
    )).await.expect_err("should fail");

    assert!(matches!(err, SubmitError::LineNotFound { .. }));
}

/// 8. Submit with negative counted_qty → NegativeCountedQty error.
#[tokio::test]
#[serial]
async fn test_submit_negative_counted_qty() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, line_id, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB08", "S-08", 20).await;

    let err = submit_cycle_count(&pool, &submit_req(
        task_id, &tenant, "submit-08",
        vec![SubmitLineInput { line_id, counted_qty: -1 }],
    )).await.expect_err("should fail");

    assert!(matches!(err, SubmitError::NegativeCountedQty { .. }));
}

/// 9. Outbox event emitted with correct event_type after submit.
#[tokio::test]
#[serial]
async fn test_submit_emits_outbox_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, line_id, _) = setup_task_with_stock(&pool, &tenant, wh, "SUB09", "S-09", 40).await;

    submit_cycle_count(&pool, &submit_req(
        task_id, &tenant, "submit-09",
        vec![SubmitLineInput { line_id, counted_qty: 38 }],
    )).await.expect("submit");

    let row: (String,) = sqlx::query_as(
        "SELECT event_type FROM inv_outbox WHERE tenant_id = $1 AND aggregate_type = 'cycle_count_task'"
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox row must exist");

    assert_eq!(row.0, "inventory.cycle_count_submitted");
}

/// 10. Submit task with zero lines (empty location full scope) → succeeds.
#[tokio::test]
#[serial]
async fn test_submit_zero_line_task() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    // Empty location — no stock at all
    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, wh, "S-10"))
        .await
        .expect("create location");

    let task = create_cycle_count_task(&pool, &CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id: wh,
        location_id: loc.id,
        scope: TaskScope::Full,
        item_ids: vec![],
    }).await.expect("create task");

    assert_eq!(task.line_count, 0);

    let (result, _) = submit_cycle_count(&pool, &submit_req(
        task.task_id, &tenant, "submit-10", vec![],
    )).await.expect("submit empty task");

    assert_eq!(result.status, "submitted");
    assert_eq!(result.line_count, 0);
    assert!(result.lines.is_empty());
}
