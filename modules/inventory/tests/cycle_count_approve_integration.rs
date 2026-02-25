//! Integration tests for cycle count approve mutation (bd-opin).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Approve submitted task → status = approved, adjustments created for variances
//! 2. Zero-variance lines produce no adjustment entries
//! 3. Idempotent approve (same key, same body) → is_replay = true, same result
//! 4. Conflicting idempotency key (different body) → error
//! 5. Approve non-submitted task → TaskNotSubmitted error
//! 6. Approve task that doesn't exist → TaskNotFound error
//! 7. Outbox contains inventory.cycle_count_approved event
//! 8. Outbox contains inventory.adjusted event per non-zero variance line
//! 9. item_on_hand updated after approval (shrinkage decrements on_hand)

use inventory_rs::domain::{
    cycle_count::{
        approve_service::{approve_cycle_count, ApproveError, ApproveRequest},
        submit_service::{submit_cycle_count, SubmitLineInput, SubmitRequest},
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
    format!("ca-tenant-{}", Uuid::new_v4())
}

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Approve Test Item {}", sku),
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
        name: format!("Approve Test Location {}", code),
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

/// Setup a task and submit it. Returns (task_id, line_id, item_id, location_id).
async fn setup_submitted_task(
    pool: &sqlx::PgPool,
    tenant: &str,
    warehouse_id: Uuid,
    sku_suffix: &str,
    loc_code: &str,
    on_hand_qty: i64,
    counted_qty: i64,
) -> (Uuid, Uuid, Uuid, Uuid) {
    let item = ItemRepo::create(pool, &make_item_req(tenant, &format!("CA-{}", sku_suffix)))
        .await
        .expect("create item");

    let loc = LocationRepo::create(pool, &make_location_req(tenant, warehouse_id, loc_code))
        .await
        .expect("create location");

    process_receipt(
        pool,
        &receipt_req(tenant, item.id, warehouse_id, loc.id, on_hand_qty, &format!("ca-rcpt-{}", sku_suffix)),
        None,
    )
    .await
    .expect("receipt");

    let task = create_cycle_count_task(
        pool,
        &CreateTaskRequest {
            tenant_id: tenant.to_string(),
            warehouse_id,
            location_id: loc.id,
            scope: TaskScope::Full,
            item_ids: vec![],
        },
    )
    .await
    .expect("create task");

    let line_id = task.lines[0].line_id;

    submit_cycle_count(
        pool,
        &SubmitRequest {
            task_id: task.task_id,
            tenant_id: tenant.to_string(),
            idempotency_key: format!("ca-submit-{}", sku_suffix),
            lines: vec![SubmitLineInput { line_id, counted_qty }],
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("submit");

    (task.task_id, line_id, item.id, loc.id)
}

fn approve_req(task_id: Uuid, tenant_id: &str, key: &str) -> ApproveRequest {
    ApproveRequest {
        task_id,
        tenant_id: tenant_id.to_string(),
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

/// 1. Approve submitted task → status = approved, adjustment created for variance.
#[tokio::test]
#[serial]
async fn test_approve_creates_adjustment_for_variance() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    // 50 on hand, counted 45 → variance = -5 (shrinkage)
    let (task_id, _line_id, item_id, _loc_id) =
        setup_submitted_task(&pool, &tenant, wh, "APR01", "A-01", 50, 45).await;

    let req = approve_req(task_id, &tenant, "approve-01");
    let (result, is_replay) = approve_cycle_count(&pool, &req).await.expect("approve");

    assert!(!is_replay, "first call is not a replay");
    assert_eq!(result.status, "approved");
    assert_eq!(result.task_id, task_id);
    assert_eq!(result.line_count, 1);
    assert_eq!(result.adjustment_count, 1, "one non-zero variance → one adjustment");

    let line = &result.lines[0];
    assert_eq!(line.item_id, item_id);
    assert_eq!(line.expected_qty, 50);
    assert_eq!(line.counted_qty, 45);
    assert_eq!(line.variance_qty, -5);
    assert!(line.adjustment_id.is_some(), "adjustment created for non-zero variance");

    // Verify task status in DB
    let status: String = sqlx::query_scalar("SELECT status::TEXT FROM cycle_count_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(&pool)
        .await
        .expect("fetch status");
    assert_eq!(status, "approved");
}

/// 2. Zero-variance lines produce no adjustment entries.
#[tokio::test]
#[serial]
async fn test_approve_zero_variance_no_adjustment() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    // 30 on hand, counted 30 → variance = 0
    let (task_id, _line_id, item_id, _loc_id) =
        setup_submitted_task(&pool, &tenant, wh, "APR02", "A-02", 30, 30).await;

    let req = approve_req(task_id, &tenant, "approve-02");
    let (result, is_replay) = approve_cycle_count(&pool, &req).await.expect("approve");

    assert!(!is_replay);
    assert_eq!(result.status, "approved");
    assert_eq!(result.adjustment_count, 0, "zero variance → no adjustments");
    assert_eq!(result.line_count, 1);

    let line = &result.lines[0];
    assert_eq!(line.item_id, item_id);
    assert_eq!(line.variance_qty, 0);
    assert!(line.adjustment_id.is_none(), "no adjustment for zero variance");
}

/// 3. Idempotent approve (same key, same body) → is_replay = true, same result.
#[tokio::test]
#[serial]
async fn test_approve_idempotent_replay() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, _line_id, _item_id, _loc_id) =
        setup_submitted_task(&pool, &tenant, wh, "APR03", "A-03", 20, 18).await;

    let req = approve_req(task_id, &tenant, "approve-03");

    let (result1, replay1) = approve_cycle_count(&pool, &req).await.expect("first approve");
    assert!(!replay1);

    let (result2, replay2) = approve_cycle_count(&pool, &req).await.expect("second approve");
    assert!(replay2, "second call with same key is a replay");
    assert_eq!(result1.task_id, result2.task_id);
    assert_eq!(result1.adjustment_count, result2.adjustment_count);
    assert_eq!(result1.lines[0].adjustment_id, result2.lines[0].adjustment_id);
}

/// 4. Conflicting idempotency key (different task_id) → error.
#[tokio::test]
#[serial]
async fn test_approve_idempotency_conflict() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id_a, _, _, _) =
        setup_submitted_task(&pool, &tenant, wh, "APR04A", "A-04A", 10, 10).await;
    let (task_id_b, _, _, _) =
        setup_submitted_task(&pool, &tenant, wh, "APR04B", "A-04B", 10, 10).await;

    let req_a = approve_req(task_id_a, &tenant, "approve-04-conflict");
    approve_cycle_count(&pool, &req_a).await.expect("first approve");

    // Same key, different task_id → conflict
    let req_b = approve_req(task_id_b, &tenant, "approve-04-conflict");
    let err = approve_cycle_count(&pool, &req_b)
        .await
        .expect_err("should be a conflict");
    assert!(
        matches!(err, ApproveError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got {:?}",
        err
    );
}

/// 5. Approve non-submitted task (still open) → TaskNotSubmitted error.
#[tokio::test]
#[serial]
async fn test_approve_non_submitted_task_errors() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    // Create task but do NOT submit it (stays 'open')
    let item = ItemRepo::create(
        &pool,
        &make_item_req(&tenant, &format!("CA-APR05-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");
    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, wh, "A-05"))
        .await
        .expect("create location");
    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, loc.id, 5, "ca-rcpt-apr05"),
        None,
    )
    .await
    .expect("receipt");
    let task = create_cycle_count_task(
        &pool,
        &CreateTaskRequest {
            tenant_id: tenant.clone(),
            warehouse_id: wh,
            location_id: loc.id,
            scope: TaskScope::Full,
            item_ids: vec![],
        },
    )
    .await
    .expect("create task");

    let req = approve_req(task.task_id, &tenant, "approve-05");
    let err = approve_cycle_count(&pool, &req)
        .await
        .expect_err("should fail on open task");
    assert!(
        matches!(err, ApproveError::TaskNotSubmitted { .. }),
        "expected TaskNotSubmitted, got {:?}",
        err
    );
}

/// 6. Approve task that doesn't exist → TaskNotFound error.
#[tokio::test]
#[serial]
async fn test_approve_missing_task_errors() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let req = approve_req(Uuid::new_v4(), &tenant, "approve-06");
    let err = approve_cycle_count(&pool, &req)
        .await
        .expect_err("should fail for missing task");
    assert!(
        matches!(err, ApproveError::TaskNotFound),
        "expected TaskNotFound, got {:?}",
        err
    );
}

/// 7. Outbox contains inventory.cycle_count_approved event.
#[tokio::test]
#[serial]
async fn test_approve_emits_cycle_count_approved_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, _, _, _) =
        setup_submitted_task(&pool, &tenant, wh, "APR07", "A-07", 15, 12).await;

    let req = approve_req(task_id, &tenant, "approve-07");
    approve_cycle_count(&pool, &req).await.expect("approve");

    let row: Option<(String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, tenant_id
        FROM inv_outbox
        WHERE tenant_id = $1
          AND aggregate_type = 'cycle_count_task'
          AND aggregate_id = $2
          AND event_type = 'inventory.cycle_count_approved'
        "#,
    )
    .bind(&tenant)
    .bind(task_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, outbox_tenant) = row.expect("cycle_count_approved outbox row must exist");
    assert_eq!(event_type, "inventory.cycle_count_approved");
    assert_eq!(outbox_tenant, tenant);
}

/// 8. Outbox contains inventory.adjusted event for the non-zero variance line.
#[tokio::test]
#[serial]
async fn test_approve_emits_adjusted_event_for_variance() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let (task_id, _, item_id, _) =
        setup_submitted_task(&pool, &tenant, wh, "APR08", "A-08", 25, 20).await;

    let req = approve_req(task_id, &tenant, "approve-08");
    approve_cycle_count(&pool, &req).await.expect("approve");

    let row: Option<(String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, tenant_id
        FROM inv_outbox
        WHERE tenant_id = $1
          AND aggregate_type = 'inventory_item'
          AND aggregate_id = $2
          AND event_type = 'inventory.adjusted'
        "#,
    )
    .bind(&tenant)
    .bind(item_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, outbox_tenant) = row.expect("inventory.adjusted outbox row must exist");
    assert_eq!(event_type, "inventory.adjusted");
    assert_eq!(outbox_tenant, tenant);
}

/// 9. item_on_hand updated after approval (shrinkage decrements on_hand).
#[tokio::test]
#[serial]
async fn test_approve_updates_on_hand_for_shrinkage() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    // 40 on hand, counted 35 → shrinkage of 5
    let (task_id, _, item_id, loc_id) =
        setup_submitted_task(&pool, &tenant, wh, "APR09", "A-09", 40, 35).await;

    let req = approve_req(task_id, &tenant, "approve-09");
    approve_cycle_count(&pool, &req).await.expect("approve");

    // After receipt: 40. After shrinkage adjustment: 40 - 5 = 35
    let qty_on_hand: i64 = sqlx::query_scalar(
        r#"
        SELECT quantity_on_hand
        FROM item_on_hand
        WHERE tenant_id    = $1
          AND item_id      = $2
          AND warehouse_id = $3
          AND location_id  = $4
        "#,
    )
    .bind(&tenant)
    .bind(item_id)
    .bind(wh)
    .bind(loc_id)
    .fetch_one(&pool)
    .await
    .expect("on_hand query");

    assert_eq!(qty_on_hand, 35, "on_hand should be 35 after -5 variance applied");
}
