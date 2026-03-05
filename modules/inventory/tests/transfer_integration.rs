//! Integration tests for inter-warehouse stock transfers (bd-jd78).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Happy path: paired ledger entries, FIFO consumption, new destination layer, outbox
//! 2. Source on-hand decrements; destination on-hand increments atomically
//! 3. Idempotency: replay returns stored result, no extra rows written
//! 4. Guard: insufficient quantity rejected
//! 5. Guard: inactive item rejected
//! 6. Guard: same warehouse rejected
//! 7. Guard: zero quantity rejected (stateless)
//! 8. FIFO: multi-layer source consumed oldest-first
//! 9. Reservations remain consistent (reserved qty not transferable)

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    reservation_service::{process_reserve, ReserveRequest},
    transfer_service::{process_transfer, TransferError, TransferRequest},
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
        .expect("Failed to run inventory migrations");
    pool
}

fn make_item(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Transfer Test Item {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn receipt(
    tenant_id: &str,
    item_id: Uuid,
    wh: Uuid,
    qty: i64,
    cost: i64,
    key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: wh,
        location_id: None,
        quantity: qty,
        unit_cost_minor: cost,
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

fn transfer(
    tenant_id: &str,
    item_id: Uuid,
    from: Uuid,
    to: Uuid,
    qty: i64,
    key: &str,
) -> TransferRequest {
    TransferRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        from_warehouse_id: from,
        to_warehouse_id: to,
        quantity: qty,
        currency: "usd".to_string(),
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_transfers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
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
    sqlx::query("DELETE FROM layer_consumptions WHERE layer_id IN (SELECT id FROM inventory_layers WHERE tenant_id = $1)").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inventory_reservations WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
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

async fn get_on_hand(pool: &sqlx::PgPool, tenant_id: &str, item_id: Uuid, wh: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(quantity_on_hand, 0) FROM item_on_hand WHERE tenant_id=$1 AND item_id=$2 AND warehouse_id=$3 AND location_id IS NULL"
    )
    .bind(tenant_id).bind(item_id).bind(wh)
    .fetch_optional(pool).await.unwrap().unwrap_or(0)
}

async fn get_available_bucket(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    wh: Uuid,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(quantity_on_hand, 0) FROM item_on_hand_by_status WHERE tenant_id=$1 AND item_id=$2 AND warehouse_id=$3 AND status='available'"
    )
    .bind(tenant_id).bind(item_id).bind(wh)
    .fetch_optional(pool).await.unwrap().unwrap_or(0)
}

// ============================================================================
// Test 1: Happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_happy_path_paired_ledger_fifo_layer_outbox() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-{}", Uuid::new_v4());
    let wh_src = Uuid::new_v4();
    let wh_dst = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-001"))
        .await
        .unwrap();

    // Receive 50 units at $10 in source warehouse
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            50,
            1000,
            &format!("rcv-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();

    // Transfer 20 units to destination
    let req = transfer(
        &tid,
        item.id,
        wh_src,
        wh_dst,
        20,
        &format!("xfr-{}", Uuid::new_v4()),
    );
    let (result, is_replay) = process_transfer(&pool, &req, None)
        .await
        .expect("transfer should succeed");

    assert!(!is_replay);
    assert_eq!(result.quantity, 20);
    assert_eq!(result.total_cost_minor, 20 * 1000);
    assert_eq!(result.consumed_layers.len(), 1);
    assert_eq!(result.consumed_layers[0].quantity, 20);

    // Source on-hand: 50 - 20 = 30
    assert_eq!(get_on_hand(&pool, &tid, item.id, wh_src).await, 30);
    assert_eq!(get_available_bucket(&pool, &tid, item.id, wh_src).await, 30);

    // Destination on-hand: 0 + 20 = 20
    assert_eq!(get_on_hand(&pool, &tid, item.id, wh_dst).await, 20);
    assert_eq!(get_available_bucket(&pool, &tid, item.id, wh_dst).await, 20);

    // Both ledger legs written
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id=$1 AND item_id=$2 AND entry_type IN ('transfer_out','transfer_in')"
    ).bind(&tid).bind(item.id).fetch_one(&pool).await.unwrap();
    assert_eq!(ledger_count, 2);

    // transfer_out is negative; transfer_in is positive
    let (out_qty, in_qty): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT quantity FROM inventory_ledger WHERE id=$1), (SELECT quantity FROM inventory_ledger WHERE id=$2)"
    ).bind(result.issue_ledger_id).bind(result.receipt_ledger_id)
    .fetch_one(&pool).await.unwrap();
    assert_eq!(out_qty, -20);
    assert_eq!(in_qty, 20);

    // Shared transfer_ref (reference_id) on both legs
    let refs: Vec<String> = sqlx::query_scalar(
        "SELECT reference_id FROM inventory_ledger WHERE tenant_id=$1 AND entry_type IN ('transfer_out','transfer_in') ORDER BY id"
    ).bind(&tid).fetch_all(&pool).await.unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0], refs[1], "both legs must share transfer_ref");
    assert_eq!(refs[0], result.transfer_id.to_string());

    // New FIFO layer created at destination
    let dest_layer_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_layers WHERE tenant_id=$1 AND item_id=$2 AND warehouse_id=$3"
    ).bind(&tid).bind(item.id).bind(wh_dst).fetch_one(&pool).await.unwrap();
    assert_eq!(dest_layer_count, 1);

    // Outbox event emitted
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id=$1 AND event_type='inventory.transfer_completed'"
    ).bind(&tid).fetch_one(&pool).await.unwrap();
    assert_eq!(outbox_count, 1);

    // inv_transfers business record
    let transfer_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inv_transfers WHERE tenant_id=$1 AND id=$2")
            .bind(&tid)
            .bind(result.transfer_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(transfer_count, 1);

    cleanup(&pool, &tid).await;
}

// ============================================================================
// Test 2: Idempotency replay
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_idempotency_replay_returns_same_result() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-idem-{}", Uuid::new_v4());
    let wh_src = Uuid::new_v4();
    let wh_dst = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-IDEM"))
        .await
        .unwrap();
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            30,
            500,
            &format!("rcv-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();

    let key = format!("xfr-idem-{}", Uuid::new_v4());
    let req = transfer(&tid, item.id, wh_src, wh_dst, 10, &key);
    let (r1, replay1) = process_transfer(&pool, &req, None).await.unwrap();
    let (r2, replay2) = process_transfer(&pool, &req, None).await.unwrap();

    assert!(!replay1);
    assert!(replay2);
    assert_eq!(r1.transfer_id, r2.transfer_id);
    assert_eq!(r1.event_id, r2.event_id);
    assert_eq!(r1.quantity, r2.quantity);

    // No duplicate ledger rows
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id=$1 AND entry_type IN ('transfer_out','transfer_in')"
    ).bind(&tid).fetch_one(&pool).await.unwrap();
    assert_eq!(ledger_count, 2);

    cleanup(&pool, &tid).await;
}

// ============================================================================
// Test 3: Insufficient quantity rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_insufficient_quantity_rejected() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-insuff-{}", Uuid::new_v4());
    let wh_src = Uuid::new_v4();
    let wh_dst = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-INS"))
        .await
        .unwrap();
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            5,
            1000,
            &format!("rcv-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();

    let req = transfer(
        &tid,
        item.id,
        wh_src,
        wh_dst,
        10,
        &format!("xfr-{}", Uuid::new_v4()),
    );
    let err = process_transfer(&pool, &req, None).await.unwrap_err();
    assert!(matches!(
        err,
        TransferError::InsufficientQuantity {
            requested: 10,
            available: 5
        }
    ));

    cleanup(&pool, &tid).await;
}

// ============================================================================
// Test 4: Inactive item rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_inactive_item_rejected() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-inactive-{}", Uuid::new_v4());
    let wh_src = Uuid::new_v4();
    let wh_dst = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-INACT"))
        .await
        .unwrap();
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            20,
            500,
            &format!("rcv-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();
    ItemRepo::deactivate(&pool, item.id, &tid).await.unwrap();

    let req = transfer(
        &tid,
        item.id,
        wh_src,
        wh_dst,
        5,
        &format!("xfr-{}", Uuid::new_v4()),
    );
    let err = process_transfer(&pool, &req, None).await.unwrap_err();
    assert!(matches!(err, TransferError::Guard(_)));

    cleanup(&pool, &tid).await;
}

// ============================================================================
// Test 5: Same-warehouse rejected (stateless)
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_same_warehouse_rejected() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-same-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-SAME"))
        .await
        .unwrap();

    let req = transfer(&tid, item.id, wh, wh, 5, &format!("xfr-{}", Uuid::new_v4()));
    let err = process_transfer(&pool, &req, None).await.unwrap_err();
    assert!(matches!(err, TransferError::SameWarehouse));

    cleanup(&pool, &tid).await;
}

// ============================================================================
// Test 6: FIFO multi-layer consumption (oldest first)
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_fifo_multi_layer_oldest_first() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-fifo-{}", Uuid::new_v4());
    let wh_src = Uuid::new_v4();
    let wh_dst = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-FIFO"))
        .await
        .unwrap();

    // Layer 1: 10 units @ $5 (older)
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            10,
            500,
            &format!("rcv1-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();
    // Layer 2: 15 units @ $8 (newer)
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            15,
            800,
            &format!("rcv2-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();

    // Transfer 12 units — should consume all of layer 1 (10) + 2 from layer 2
    let req = transfer(
        &tid,
        item.id,
        wh_src,
        wh_dst,
        12,
        &format!("xfr-{}", Uuid::new_v4()),
    );
    let (result, _) = process_transfer(&pool, &req, None).await.unwrap();

    assert_eq!(result.consumed_layers.len(), 2);
    assert_eq!(result.consumed_layers[0].quantity, 10);
    assert_eq!(result.consumed_layers[0].unit_cost_minor, 500);
    assert_eq!(result.consumed_layers[1].quantity, 2);
    assert_eq!(result.consumed_layers[1].unit_cost_minor, 800);

    // Total cost = 10*500 + 2*800 = 5000 + 1600 = 6600
    assert_eq!(result.total_cost_minor, 6600);

    // Source: 25 - 12 = 13 remaining
    assert_eq!(get_on_hand(&pool, &tid, item.id, wh_src).await, 13);
    // Destination: 12 received
    assert_eq!(get_on_hand(&pool, &tid, item.id, wh_dst).await, 12);

    cleanup(&pool, &tid).await;
}

// ============================================================================
// Test 7: Reservations reduce available quantity for transfer
// ============================================================================

#[tokio::test]
#[serial]
async fn transfer_reserved_qty_not_transferable() {
    let pool = setup_db().await;
    let tid = format!("test-xfer-rsv-{}", Uuid::new_v4());
    let wh_src = Uuid::new_v4();
    let wh_dst = Uuid::new_v4();
    cleanup(&pool, &tid).await;

    let item = ItemRepo::create(&pool, &make_item(&tid, "SKU-XFR-RSV"))
        .await
        .unwrap();
    process_receipt(
        &pool,
        &receipt(
            &tid,
            item.id,
            wh_src,
            20,
            1000,
            &format!("rcv-{}", Uuid::new_v4()),
        ),
        None,
    )
    .await
    .unwrap();

    // Reserve 15 units
    process_reserve(
        &pool,
        &ReserveRequest {
            tenant_id: tid.clone(),
            item_id: item.id,
            warehouse_id: wh_src,
            quantity: 15,
            reference_type: Some("sales_order".to_string()),
            reference_id: Some("SO-001".to_string()),
            expires_at: None,
            idempotency_key: format!("rsv-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .unwrap();

    // Only 5 units available; trying to transfer 10 should fail
    let req = transfer(
        &tid,
        item.id,
        wh_src,
        wh_dst,
        10,
        &format!("xfr-{}", Uuid::new_v4()),
    );
    let err = process_transfer(&pool, &req, None).await.unwrap_err();
    assert!(matches!(
        err,
        TransferError::InsufficientQuantity {
            requested: 10,
            available: 5
        }
    ));

    // Transferring exactly 5 (available) should succeed
    let req2 = transfer(
        &tid,
        item.id,
        wh_src,
        wh_dst,
        5,
        &format!("xfr-{}", Uuid::new_v4()),
    );
    let (result, _) = process_transfer(&pool, &req2, None)
        .await
        .expect("transfer of 5 available units should succeed");
    assert_eq!(result.quantity, 5);

    cleanup(&pool, &tid).await;
}
