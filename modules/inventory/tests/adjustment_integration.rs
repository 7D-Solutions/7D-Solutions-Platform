//! Integration tests for stock adjustments (bd-820g).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Positive adjustment: on_hand and available bucket increase
//! 2. Negative adjustment: on_hand and available bucket decrease
//! 3. No-negative guard: adjustment blocked when it would make on_hand negative
//! 4. allow_negative bypasses no-negative guard
//! 5. Zero quantity_delta rejected
//! 6. Empty reason rejected
//! 7. Inactive item rejected
//! 8. Idempotency: replay returns stored result
//! 9. Idempotency conflict: same key, different body → error

use inventory_rs::domain::{
    adjust_service::{process_adjustment, AdjustError, AdjustRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
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
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string());
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

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Test Item {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        unit_cost_minor: 1000,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

fn adjust_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    delta: i64,
    reason: &str,
    key: &str,
) -> AdjustRequest {
    AdjustRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity_delta: delta,
        reason: reason.to_string(),
        allow_negative: false,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

async fn read_on_hand(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(quantity_on_hand, 0) FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND location_id IS NULL",
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_optional(pool)
    .await
    .unwrap()
    .unwrap_or(0)
}

async fn read_available_status(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(quantity_on_hand, 0) FROM item_on_hand_by_status WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND status = 'available'",
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_optional(pool)
    .await
    .unwrap()
    .unwrap_or(0)
}

// ============================================================================
// Test 1: Positive adjustment increases on_hand and available bucket
// ============================================================================

#[tokio::test]
#[serial]
async fn test_positive_adjustment_increases_on_hand() {
    let pool = setup_db().await;
    let tenant = format!("adj-t1-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-001"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, 50, "r-adj-1"),
        None,
    )
    .await
    .expect("receipt");

    let before = read_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(before, 50);

    let (result, is_replay) = process_adjustment(
        &pool,
        &adjust_req(&tenant, item.id, wh, 15, "cycle_count_correction", "adj-1"),
        None,
    )
    .await
    .expect("adjustment");

    assert!(!is_replay);
    assert_eq!(result.quantity_delta, 15);
    assert_eq!(result.reason, "cycle_count_correction");

    let after = read_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(after, 65, "on_hand must increase by delta");

    let avail = read_available_status(&pool, &tenant, item.id, wh).await;
    assert_eq!(avail, 65, "available bucket must increase by delta");
}

// ============================================================================
// Test 2: Negative adjustment decreases on_hand and available bucket
// ============================================================================

#[tokio::test]
#[serial]
async fn test_negative_adjustment_decreases_on_hand() {
    let pool = setup_db().await;
    let tenant = format!("adj-t2-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-002"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, 100, "r-adj-2"),
        None,
    )
    .await
    .expect("receipt");

    let (result, _) = process_adjustment(
        &pool,
        &adjust_req(&tenant, item.id, wh, -20, "shrinkage", "adj-2"),
        None,
    )
    .await
    .expect("negative adjustment");

    assert_eq!(result.quantity_delta, -20);

    let after = read_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(after, 80, "on_hand must decrease by abs(delta)");

    let avail = read_available_status(&pool, &tenant, item.id, wh).await;
    assert_eq!(avail, 80, "available bucket must decrease by abs(delta)");
}

// ============================================================================
// Test 3: No-negative guard blocks adjustment that would go negative
// ============================================================================

#[tokio::test]
#[serial]
async fn test_no_negative_guard_blocks_excess_negative() {
    let pool = setup_db().await;
    let tenant = format!("adj-t3-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-003"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, 30, "r-adj-3"),
        None,
    )
    .await
    .expect("receipt");

    let err = process_adjustment(
        &pool,
        &adjust_req(&tenant, item.id, wh, -50, "shrinkage", "adj-3"),
        None,
    )
    .await
    .expect_err("should be blocked by no-negative guard");

    assert!(
        matches!(err, AdjustError::NegativeOnHand { .. }),
        "expected NegativeOnHand, got {:?}",
        err
    );

    let on_hand = read_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(
        on_hand, 30,
        "on_hand must be unchanged after blocked adjustment"
    );
}

// ============================================================================
// Test 4: allow_negative bypasses no-negative guard
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allow_negative_bypasses_guard() {
    let pool = setup_db().await;
    let tenant = format!("adj-t4-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-004"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, 10, "r-adj-4"),
        None,
    )
    .await
    .expect("receipt");

    let mut req = adjust_req(&tenant, item.id, wh, -50, "write_off", "adj-4");
    req.allow_negative = true;

    let (result, _) = process_adjustment(&pool, &req, None)
        .await
        .expect("adjustment with allow_negative should succeed");

    assert_eq!(result.quantity_delta, -50);

    let after = read_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(
        after, -40,
        "on_hand can go negative when allow_negative = true"
    );
}

// ============================================================================
// Test 5: Zero quantity_delta rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_zero_delta_rejected() {
    let pool = setup_db().await;
    let tenant = format!("adj-t5-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-005"))
        .await
        .expect("create item");

    let err = process_adjustment(
        &pool,
        &adjust_req(&tenant, item.id, wh, 0, "shrinkage", "adj-5"),
        None,
    )
    .await
    .expect_err("zero delta should be rejected");

    assert!(
        matches!(err, AdjustError::Guard(_)),
        "expected Guard error for zero delta"
    );
}

// ============================================================================
// Test 6: Empty reason rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_empty_reason_rejected() {
    let pool = setup_db().await;
    let tenant = format!("adj-t6-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-006"))
        .await
        .expect("create item");

    let err = process_adjustment(
        &pool,
        &adjust_req(&tenant, item.id, wh, 10, "  ", "adj-6"),
        None,
    )
    .await
    .expect_err("empty reason should be rejected");

    assert!(
        matches!(err, AdjustError::Guard(_)),
        "expected Guard error for empty reason"
    );
}

// ============================================================================
// Test 7: Inactive item rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_inactive_item_rejected() {
    let pool = setup_db().await;
    let tenant = format!("adj-t7-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-007"))
        .await
        .expect("create item");

    sqlx::query("UPDATE items SET active = false WHERE id = $1")
        .bind(item.id)
        .execute(&pool)
        .await
        .expect("deactivate item");

    let err = process_adjustment(
        &pool,
        &adjust_req(&tenant, item.id, wh, 10, "shrinkage", "adj-7"),
        None,
    )
    .await
    .expect_err("inactive item should be rejected");

    assert!(
        matches!(err, AdjustError::Guard(_)),
        "expected Guard error for inactive item"
    );
}

// ============================================================================
// Test 8: Idempotency replay returns stored result
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency_replay() {
    let pool = setup_db().await;
    let tenant = format!("adj-t8-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-008"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, 100, "r-adj-8"),
        None,
    )
    .await
    .expect("receipt");

    let req = adjust_req(&tenant, item.id, wh, 25, "cycle_count", "adj-8-idem");

    let (first, is_replay_1) = process_adjustment(&pool, &req, None)
        .await
        .expect("first call");
    assert!(!is_replay_1);

    let (second, is_replay_2) = process_adjustment(&pool, &req, None).await.expect("replay");
    assert!(
        is_replay_2,
        "second call with same key+body should be a replay"
    );
    assert_eq!(first.adjustment_id, second.adjustment_id);

    // on_hand should only increase once
    let on_hand = read_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(on_hand, 125, "replayed adjustment must not double-count");
}

// ============================================================================
// Test 9: Idempotency conflict: same key, different body
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency_conflict() {
    let pool = setup_db().await;
    let tenant = format!("adj-t9-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "ADJ-009"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, wh, 100, "r-adj-9"),
        None,
    )
    .await
    .expect("receipt");

    let key = "conflict-key-adj";
    let req1 = adjust_req(&tenant, item.id, wh, 10, "shrinkage", key);
    process_adjustment(&pool, &req1, None)
        .await
        .expect("first call");

    let mut req2 = adjust_req(&tenant, item.id, wh, 99, "different_reason", key);
    req2.idempotency_key = key.to_string();

    let err = process_adjustment(&pool, &req2, None)
        .await
        .expect_err("conflicting idempotency key should error");

    assert!(
        matches!(err, AdjustError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got {:?}",
        err
    );
}
