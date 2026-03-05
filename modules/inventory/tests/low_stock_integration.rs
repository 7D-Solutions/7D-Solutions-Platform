//! Integration tests for low-stock signal emission (bd-3lwn).
//!
//! Tests run against a real PostgreSQL database (inventory_db).
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Issue drops below reorder_point → signal emitted to inv_outbox
//! 2. Second issue while still below → no new signal (dedup)
//! 3. Adjustment brings stock above reorder_point → state resets
//! 4. Third issue drops below again → new signal emitted (re-arm)
//! 5. No reorder policy → no signal
//! 6. Adjustment drops below reorder_point → signal emitted

use inventory_rs::domain::{
    adjust_service::{process_adjustment, AdjustRequest},
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    locations::{CreateLocationRequest, LocationRepo},
    receipt_service::{process_receipt, ReceiptRequest},
    reorder::models::{CreateReorderPolicyRequest, ReorderPolicyRepo},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=disable".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

fn unique_tenant() -> String {
    format!("ls-tenant-{}", Uuid::new_v4())
}

fn make_item(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("LS Item {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn make_location(tenant_id: &str, wh: Uuid, code: &str) -> CreateLocationRequest {
    CreateLocationRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id: wh,
        code: code.to_string(),
        name: format!("LS Loc {}", code),
        description: None,
    }
}

fn receipt(
    tenant_id: &str,
    item_id: Uuid,
    wh: Uuid,
    loc_id: Uuid,
    qty: i64,
    key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: wh,
        location_id: Some(loc_id),
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

fn issue(
    tenant_id: &str,
    item_id: Uuid,
    wh: Uuid,
    loc_id: Uuid,
    qty: i64,
    key: &str,
) -> IssueRequest {
    IssueRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: wh,
        location_id: Some(loc_id),
        quantity: qty,
        currency: "usd".to_string(),
        source_module: "test".to_string(),
        source_type: "test_order".to_string(),
        source_id: key.to_string(),
        source_line_id: None,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    }
}

fn adjustment(
    tenant_id: &str,
    item_id: Uuid,
    wh: Uuid,
    loc_id: Uuid,
    delta: i64,
    key: &str,
) -> AdjustRequest {
    AdjustRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: wh,
        location_id: Some(loc_id),
        quantity_delta: delta,
        reason: "test_adjustment".to_string(),
        allow_negative: false,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

async fn count_low_stock_signals(pool: &sqlx::PgPool, tenant_id: &str, item_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM inv_outbox \
         WHERE tenant_id = $1 AND aggregate_id = $2 \
           AND event_type = 'inventory.low_stock_triggered'",
    )
    .bind(tenant_id)
    .bind(item_id.to_string())
    .fetch_one(pool)
    .await
    .expect("count query")
}

async fn get_below_threshold(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    location_id: Option<Uuid>,
) -> Option<bool> {
    match location_id {
        None => sqlx::query_scalar(
            "SELECT below_threshold FROM inv_low_stock_state \
                 WHERE tenant_id = $1 AND item_id = $2 AND location_id IS NULL",
        )
        .bind(tenant_id)
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .expect("state query"),
        Some(loc_id) => sqlx::query_scalar(
            "SELECT below_threshold FROM inv_low_stock_state \
                 WHERE tenant_id = $1 AND item_id = $2 AND location_id = $3",
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(loc_id)
        .fetch_optional(pool)
        .await
        .expect("state query"),
    }
}

// ============================================================================
// Tests
// ============================================================================

/// 1. Issue drops below reorder_point → exactly one signal in outbox.
#[tokio::test]
#[serial]
async fn test_issue_crossing_below_emits_signal() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant, "LS-01"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &make_location(&tenant, wh, "L-01"))
        .await
        .unwrap();

    // Policy: reorder_point = 20, scoped to this location
    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            location_id: Some(loc.id),
            reorder_point: 20,
            safety_stock: 5,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    // Receive 30 units — well above threshold
    process_receipt(
        &pool,
        &receipt(&tenant, item.id, wh, loc.id, 30, "r-ls01"),
        None,
    )
    .await
    .unwrap();

    // Issue 15 units: available = 15, below reorder_point 20 → signal expected
    process_issue(
        &pool,
        &issue(&tenant, item.id, wh, loc.id, 15, "i-ls01"),
        None,
    )
    .await
    .unwrap();

    let signals = count_low_stock_signals(&pool, &tenant, item.id).await;
    assert_eq!(signals, 1, "expected exactly 1 low-stock signal");

    let state = get_below_threshold(&pool, &tenant, item.id, Some(loc.id)).await;
    assert_eq!(state, Some(true), "state should be below_threshold");
}

/// 2. Second issue while still below → no new signal (dedup).
#[tokio::test]
#[serial]
async fn test_second_issue_while_below_no_duplicate() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant, "LS-02"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &make_location(&tenant, wh, "L-02"))
        .await
        .unwrap();

    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            location_id: Some(loc.id),
            reorder_point: 20,
            safety_stock: 5,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    process_receipt(
        &pool,
        &receipt(&tenant, item.id, wh, loc.id, 30, "r-ls02"),
        None,
    )
    .await
    .unwrap();

    // First issue — crosses below → signal 1
    process_issue(
        &pool,
        &issue(&tenant, item.id, wh, loc.id, 12, "i-ls02a"),
        None,
    )
    .await
    .unwrap();
    // Second issue — still below → no new signal
    process_issue(
        &pool,
        &issue(&tenant, item.id, wh, loc.id, 5, "i-ls02b"),
        None,
    )
    .await
    .unwrap();

    let signals = count_low_stock_signals(&pool, &tenant, item.id).await;
    assert_eq!(
        signals, 1,
        "should still be exactly 1 signal after second issue below threshold"
    );
}

/// 3. Adjustment brings stock above → state resets, no signal.
/// 4. Third issue drops below again → new signal (re-arm test).
#[tokio::test]
#[serial]
async fn test_recovery_and_rearm() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant, "LS-03"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &make_location(&tenant, wh, "L-03"))
        .await
        .unwrap();

    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            location_id: Some(loc.id),
            reorder_point: 20,
            safety_stock: 5,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    process_receipt(
        &pool,
        &receipt(&tenant, item.id, wh, loc.id, 50, "r-ls03a"),
        None,
    )
    .await
    .unwrap();

    // Drop below → signal 1 (50 - 35 = 15, below reorder_point 20)
    process_issue(
        &pool,
        &issue(&tenant, item.id, wh, loc.id, 35, "i-ls03a"),
        None,
    )
    .await
    .unwrap();
    let signals_after_first = count_low_stock_signals(&pool, &tenant, item.id).await;
    assert_eq!(signals_after_first, 1);

    // Recover above with a receipt (+30 → available = 45).
    // Using receipt (not adjustment) so FIFO layers are replenished for the next issue.
    process_receipt(
        &pool,
        &receipt(&tenant, item.id, wh, loc.id, 30, "r-ls03b"),
        None,
    )
    .await
    .unwrap();

    let state_after_recovery = get_below_threshold(&pool, &tenant, item.id, Some(loc.id)).await;
    assert_eq!(
        state_after_recovery,
        Some(false),
        "state should be re-armed after recovery"
    );

    // Drop below again → signal 2 (45 - 30 = 15, below reorder_point 20)
    process_issue(
        &pool,
        &issue(&tenant, item.id, wh, loc.id, 30, "i-ls03b"),
        None,
    )
    .await
    .unwrap();
    let signals_after_rearm = count_low_stock_signals(&pool, &tenant, item.id).await;
    assert_eq!(
        signals_after_rearm, 2,
        "should have 2 signals: initial + re-armed crossing"
    );
}

/// 5. No reorder policy → no signal emitted.
#[tokio::test]
#[serial]
async fn test_no_policy_no_signal() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant, "LS-04"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &make_location(&tenant, wh, "L-04"))
        .await
        .unwrap();

    // No reorder policy for this item
    process_receipt(
        &pool,
        &receipt(&tenant, item.id, wh, loc.id, 30, "r-ls04"),
        None,
    )
    .await
    .unwrap();
    process_issue(
        &pool,
        &issue(&tenant, item.id, wh, loc.id, 25, "i-ls04"),
        None,
    )
    .await
    .unwrap();

    let signals = count_low_stock_signals(&pool, &tenant, item.id).await;
    assert_eq!(signals, 0, "no signal when no reorder policy exists");
}

/// 6. Adjustment drops below reorder_point → signal emitted.
#[tokio::test]
#[serial]
async fn test_adjustment_crossing_below_emits_signal() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant, "LS-05"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &make_location(&tenant, wh, "L-05"))
        .await
        .unwrap();

    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            location_id: Some(loc.id),
            reorder_point: 15,
            safety_stock: 0,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    process_receipt(
        &pool,
        &receipt(&tenant, item.id, wh, loc.id, 25, "r-ls05"),
        None,
    )
    .await
    .unwrap();

    // Negative adjustment: −15 → available = 10, below reorder_point 15
    process_adjustment(
        &pool,
        &adjustment(&tenant, item.id, wh, loc.id, -15, "a-ls05"),
        None,
    )
    .await
    .unwrap();

    let signals = count_low_stock_signals(&pool, &tenant, item.id).await;
    assert_eq!(signals, 1, "adjustment crossing below should emit signal");
}
