//! E2E Test: Inventory Receipt Flow (bd-qwm4)
//!
//! ## Coverage
//! 1. inventory_receipt_creates_ledger_layer_outbox — atomicity: all 3 rows committed together
//! 2. inventory_receipt_idempotency_replay           — duplicate key returns stored result
//! 3. inventory_receipt_guard_inactive_item          — inactive item returns Guard error
//! 4. inventory_receipt_guard_zero_quantity          — qty = 0 returns Guard error
//! 5. inventory_receipt_guard_zero_cost              — cost = 0 returns Guard error
//!
//! ## Setup
//! Set INVENTORY_DATABASE_URL (or DATABASE_URL) to the inventory DB connection string.

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptError, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB — is INVENTORY_DATABASE_URL set?");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

fn test_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "E2E Test Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    }
}

fn test_receipt_req(tenant_id: &str, item_id: Uuid) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: Uuid::new_v4(),
        location_id: None,
        quantity: 100,
        unit_cost_minor: 25_00, // $25.00
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("e2e-idem-{}", Uuid::new_v4()),
        correlation_id: Some("e2e-corr".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_receipt_creates_ledger_layer_outbox() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-RCV-001"))
        .await
        .expect("create item");

    let req = test_receipt_req(&tenant_id, item.id);
    let (result, is_replay) = process_receipt(&pool, &req)
        .await
        .expect("receipt must succeed");

    assert!(!is_replay, "must be a new receipt");
    assert_eq!(result.quantity, 100);
    assert_eq!(result.unit_cost_minor, 2500);

    // Ledger row must exist with source_event_id matching outbox event_id
    let ledger: (i64, uuid::Uuid) = sqlx::query_as(
        "SELECT quantity, source_event_id FROM inventory_ledger WHERE entry_id = $1",
    )
    .bind(result.receipt_line_id)
    .fetch_one(&pool)
    .await
    .expect("ledger row must exist");
    assert_eq!(ledger.0, 100);
    assert_eq!(ledger.1, result.event_id);

    // FIFO layer must have quantity_remaining == quantity_received
    let (qty_recv, qty_rem): (i64, i64) = sqlx::query_as(
        "SELECT quantity_received, quantity_remaining FROM inventory_layers WHERE id = $1",
    )
    .bind(result.layer_id)
    .fetch_one(&pool)
    .await
    .expect("FIFO layer must exist");
    assert_eq!(qty_recv, 100);
    assert_eq!(
        qty_rem, 100,
        "remaining should equal received on fresh layer"
    );

    // Outbox event must carry correct event_type
    let event_type: String =
        sqlx::query_scalar("SELECT event_type FROM inv_outbox WHERE event_id = $1")
            .bind(result.event_id)
            .fetch_one(&pool)
            .await
            .expect("outbox event must exist");
    assert_eq!(event_type, "inventory.item_received");

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_receipt_idempotency_replay() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-IDEM-001"))
        .await
        .expect("create item");

    let req = test_receipt_req(&tenant_id, item.id);

    let (r1, is_replay1) = process_receipt(&pool, &req).await.expect("first call");
    assert!(!is_replay1);

    let (r2, is_replay2) = process_receipt(&pool, &req).await.expect("second call");
    assert!(is_replay2, "second call must be a replay");

    // Stored result must be identical
    assert_eq!(r1.receipt_line_id, r2.receipt_line_id);
    assert_eq!(r1.layer_id, r2.layer_id);
    assert_eq!(r1.event_id, r2.event_id);
    assert_eq!(r1.quantity, r2.quantity);

    // No duplicate rows
    let ledger_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(ledger_count, 1, "no duplicate ledger rows after replay");

    let outbox_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(outbox_count, 1, "no duplicate outbox events after replay");

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_receipt_guard_inactive_item() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-INACT-001"))
        .await
        .expect("create item");

    ItemRepo::deactivate(&pool, item.id, &tenant_id)
        .await
        .expect("deactivate item");

    let req = test_receipt_req(&tenant_id, item.id);
    let err = process_receipt(&pool, &req)
        .await
        .expect_err("inactive item must be rejected");

    assert!(
        matches!(err, ReceiptError::Guard(_)),
        "expected Guard error, got {:?}",
        err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
async fn inventory_receipt_guard_zero_quantity() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let req = ReceiptRequest {
        quantity: 0,
        ..test_receipt_req(&tenant_id, Uuid::new_v4())
    };

    let err = process_receipt(&pool, &req)
        .await
        .expect_err("zero quantity must be rejected");
    assert!(matches!(err, ReceiptError::Guard(_)), "{:?}", err);
}

#[tokio::test]
async fn inventory_receipt_guard_zero_cost() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let req = ReceiptRequest {
        unit_cost_minor: 0,
        ..test_receipt_req(&tenant_id, Uuid::new_v4())
    };

    let err = process_receipt(&pool, &req)
        .await
        .expect_err("zero cost must be rejected");
    assert!(matches!(err, ReceiptError::Guard(_)), "{:?}", err);
}
