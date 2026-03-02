//! Integration tests for the stock receipt flow (bd-qwm4).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Happy path: ledger row + FIFO layer + outbox event created in one tx
//! 2. Idempotency: duplicate key returns stored result, no extra rows
//! 3. Guard: inactive item rejected
//! 4. Guard: zero quantity rejected
//! 5. Guard: zero cost rejected

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptError, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

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

fn create_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Test Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    }
}

fn receipt_req(tenant_id: &str, item_id: Uuid, idem_key: &str) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: Uuid::new_v4(),
        quantity: 50,
        unit_cost_minor: 10_00, // $10.00
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: idem_key.to_string(),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    }
}

// ============================================================================
// Happy-path: ledger + layer + outbox created atomically
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_creates_ledger_layer_outbox_atomically() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    // Create item
    let item_req = create_item_req(&tenant_id, "SKU-RCV-001");
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create item");

    let idem_key = format!("idem-{}", Uuid::new_v4());
    let req = receipt_req(&tenant_id, item.id, &idem_key);

    let (result, is_replay) = process_receipt(&pool, &req, None)
        .await
        .expect("receipt should succeed");

    assert!(!is_replay, "first call must not be a replay");
    assert_eq!(result.quantity, 50);
    assert_eq!(result.unit_cost_minor, 1000);
    assert_eq!(result.tenant_id, tenant_id);

    // Verify ledger row
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND entry_id = $2",
    )
    .bind(&tenant_id)
    .bind(result.receipt_line_id)
    .fetch_one(&pool)
    .await
    .expect("ledger query");
    assert_eq!(ledger_count, 1, "exactly one ledger row");

    // Verify FIFO layer
    let layer_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_layers WHERE tenant_id = $1 AND id = $2",
    )
    .bind(&tenant_id)
    .bind(result.layer_id)
    .fetch_one(&pool)
    .await
    .expect("layer query");
    assert_eq!(layer_count, 1, "exactly one FIFO layer");

    // Verify outbox event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_id = $2",
    )
    .bind(&tenant_id)
    .bind(result.event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox query");
    assert_eq!(outbox_count, 1, "exactly one outbox event");

    // Verify FIFO layer quantity_remaining = quantity_received
    let layer_qty: i64 =
        sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
            .bind(result.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer qty query");
    assert_eq!(
        layer_qty, 50,
        "FIFO layer qty_remaining should equal received"
    );

    // Cleanup
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Idempotency: duplicate key returns stored result, no extra rows
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_idempotency_replay_returns_stored_result() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item_req = create_item_req(&tenant_id, "SKU-IDEM-001");
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create item");

    let idem_key = format!("idem-{}", Uuid::new_v4());
    let req = receipt_req(&tenant_id, item.id, &idem_key);

    // First call
    let (result1, is_replay1) = process_receipt(&pool, &req, None)
        .await
        .expect("first call");
    assert!(!is_replay1);

    // Second call with same key and same body
    let (result2, is_replay2) = process_receipt(&pool, &req, None)
        .await
        .expect("second call");
    assert!(is_replay2, "second call must be a replay");
    assert_eq!(result1.receipt_line_id, result2.receipt_line_id);
    assert_eq!(result1.layer_id, result2.layer_id);
    assert_eq!(result1.event_id, result2.event_id);

    // No extra rows: still exactly one of each
    let ledger_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("ledger count");
    assert_eq!(ledger_count, 1, "no duplicate ledger rows on replay");

    // Cleanup
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Guard: inactive item rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_guard_rejects_inactive_item() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item_req = create_item_req(&tenant_id, "SKU-INACTIVE-001");
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create item");

    // Deactivate the item
    ItemRepo::deactivate(&pool, item.id, &tenant_id)
        .await
        .expect("deactivate item");

    let req = receipt_req(&tenant_id, item.id, &format!("idem-{}", Uuid::new_v4()));
    let err = process_receipt(&pool, &req, None)
        .await
        .expect_err("must fail for inactive item");

    assert!(
        matches!(err, ReceiptError::Guard(_)),
        "expected Guard error, got: {:?}",
        err
    );

    // Cleanup
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Guard: zero quantity rejected (stateless validation, no DB needed)
// ============================================================================

#[tokio::test]
async fn receipt_guard_rejects_zero_quantity() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let req = ReceiptRequest {
        quantity: 0,
        idempotency_key: format!("idem-{}", Uuid::new_v4()),
        ..receipt_req(&tenant_id, Uuid::new_v4(), "irrelevant")
    };

    let err = process_receipt(&pool, &req, None)
        .await
        .expect_err("zero qty must fail");
    assert!(matches!(err, ReceiptError::Guard(_)), "{:?}", err);
}

// ============================================================================
// Guard: zero cost rejected (stateless validation, no DB needed)
// ============================================================================

#[tokio::test]
async fn receipt_guard_rejects_zero_cost() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let req = ReceiptRequest {
        unit_cost_minor: 0,
        idempotency_key: format!("idem-{}", Uuid::new_v4()),
        ..receipt_req(&tenant_id, Uuid::new_v4(), "irrelevant")
    };

    let err = process_receipt(&pool, &req, None)
        .await
        .expect_err("zero cost must fail");
    assert!(matches!(err, ReceiptError::Guard(_)), "{:?}", err);
}

// ============================================================================
// Lot tracking: receipt creates lot row and associates lot_id to FIFO layer
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_lot_tracked_creates_lot_and_associates_layer() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    // Create lot-tracked item
    let item_req = CreateItemRequest {
        tenant_id: tenant_id.clone(),
        sku: "SKU-LOT-001".to_string(),
        name: "Lot Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::Lot,
    };
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create lot item");

    let lot_code = "LOT-2026-001";
    let idem_key = format!("idem-{}", Uuid::new_v4());
    let req = ReceiptRequest {
        lot_code: Some(lot_code.to_string()),
        ..receipt_req(&tenant_id, item.id, &idem_key)
    };

    let (result, is_replay) = process_receipt(&pool, &req, None)
        .await
        .expect("lot receipt must succeed");
    assert!(!is_replay);

    // lot_id must be set on the result
    let lot_id = result
        .lot_id
        .expect("lot_id must be set for lot-tracked receipt");

    // Verify lot row exists
    let lot_code_db: String =
        sqlx::query_scalar("SELECT lot_code FROM inventory_lots WHERE id = $1 AND tenant_id = $2")
            .bind(lot_id)
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("lot row must exist");
    assert_eq!(lot_code_db, lot_code);

    // Verify FIFO layer has lot_id set
    let layer_lot_id: Option<Uuid> =
        sqlx::query_scalar("SELECT lot_id FROM inventory_layers WHERE id = $1")
            .bind(result.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer query");
    assert_eq!(
        layer_lot_id,
        Some(lot_id),
        "layer.lot_id must match the upserted lot"
    );

    // Cleanup (serial instances first, then layers, lots, ledger, items)
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_lots WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Lot tracking: receipt without lot_code rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_lot_tracked_rejects_missing_lot_code() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item_req = CreateItemRequest {
        tenant_id: tenant_id.clone(),
        sku: "SKU-LOT-GUARD-001".to_string(),
        name: "Lot Guard Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::Lot,
    };
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create lot item");

    // No lot_code provided
    let req = receipt_req(&tenant_id, item.id, &format!("idem-{}", Uuid::new_v4()));
    let err = process_receipt(&pool, &req, None)
        .await
        .expect_err("must fail without lot_code");

    assert!(
        matches!(err, ReceiptError::LotCodeRequired),
        "expected LotCodeRequired, got: {:?}",
        err
    );

    // Cleanup
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Serial tracking: receipt creates serial instances tied to layer
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_serial_tracked_creates_serial_instances() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    // Create serial-tracked item
    let item_req = CreateItemRequest {
        tenant_id: tenant_id.clone(),
        sku: "SKU-SN-001".to_string(),
        name: "Serial Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::Serial,
    };
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create serial item");

    let quantity: i64 = 3;
    let serial_codes: Vec<String> = (0..quantity)
        .map(|i| format!("SN-{}-{:04}", Uuid::new_v4().simple(), i))
        .collect();

    let idem_key = format!("idem-{}", Uuid::new_v4());
    let req = ReceiptRequest {
        quantity,
        serial_codes: Some(serial_codes.clone()),
        ..receipt_req(&tenant_id, item.id, &idem_key)
    };

    let (result, is_replay) = process_receipt(&pool, &req, None)
        .await
        .expect("serial receipt must succeed");
    assert!(!is_replay);
    assert_eq!(result.serial_instance_ids.len(), quantity as usize);
    assert!(result.lot_id.is_none(), "serial items must not set lot_id");

    // Verify serial instances exist and are tied to the layer
    for (i, sn_id) in result.serial_instance_ids.iter().enumerate() {
        let (code, layer_id_db, status): (String, Uuid, String) = sqlx::query_as(
            "SELECT serial_code, layer_id, status FROM inventory_serial_instances WHERE id = $1",
        )
        .bind(sn_id)
        .fetch_one(&pool)
        .await
        .expect("serial instance must exist");

        assert_eq!(code, serial_codes[i]);
        assert_eq!(layer_id_db, result.layer_id);
        assert_eq!(status, "on_hand");
    }

    // Cleanup (serial instances before layers due to FK)
    sqlx::query("DELETE FROM inventory_serial_instances WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Serial tracking: mismatched serial_codes count rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn receipt_serial_tracked_rejects_count_mismatch() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item_req = CreateItemRequest {
        tenant_id: tenant_id.clone(),
        sku: "SKU-SN-GUARD-001".to_string(),
        name: "Serial Guard Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::Serial,
    };
    let item = ItemRepo::create(&pool, &item_req)
        .await
        .expect("create serial item");

    // quantity=50 but only 2 serial codes
    let req = ReceiptRequest {
        serial_codes: Some(vec!["SN-A".to_string(), "SN-B".to_string()]),
        ..receipt_req(&tenant_id, item.id, &format!("idem-{}", Uuid::new_v4()))
    };
    let err = process_receipt(&pool, &req, None)
        .await
        .expect_err("count mismatch must fail");

    assert!(
        matches!(
            err,
            ReceiptError::SerialCountMismatch {
                expected: 50,
                got: 2
            }
        ),
        "expected SerialCountMismatch, got: {:?}",
        err
    );

    // Cleanup
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}
