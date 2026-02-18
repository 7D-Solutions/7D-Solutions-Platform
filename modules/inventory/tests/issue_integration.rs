//! Integration tests for the stock issue flow (bd-8c8p).
//!
//! Tests run against a real PostgreSQL database (inventory_db on port 5442).
//! Set DATABASE_URL=postgres://inventory_user:inventory_pass@localhost:5442/inventory_db
//!
//! Coverage:
//! 1. Happy path: ledger + consumptions + layer update + on-hand + outbox atomically
//! 2. FIFO consumption across multiple layers (oldest first)
//! 3. Idempotency: duplicate key returns stored result, no extra rows
//! 4. Guard: insufficient quantity rejected
//! 5. Guard: inactive item rejected
//! 6. Guard: zero quantity rejected (stateless)
//! 7. consumed_layers carried in outbox event payload
//! 8. source_ref carried in outbox event payload

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    issue_service::{process_issue, IssueError, IssueRequest},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
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

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Issue Test Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    }
}

fn make_receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    cost: i64,
    idem: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity: qty,
        unit_cost_minor: cost,
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: idem.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
    }
}

fn make_issue_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    idem: &str,
) -> IssueRequest {
    IssueRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity: qty,
        currency: "usd".to_string(),
        source_module: "orders".to_string(),
        source_type: "sales_order".to_string(),
        source_id: format!("SO-{}", Uuid::new_v4()),
        source_line_id: Some("L1".to_string()),
        idempotency_key: idem.to_string(),
        correlation_id: Some("corr-issue-test".to_string()),
        causation_id: None,
    }
}

async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM layer_consumptions WHERE layer_id IN (SELECT id FROM inventory_layers WHERE tenant_id = $1)").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
}

// ============================================================================
// Test 1: Happy path — ledger + consumptions + layer update + on-hand + outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn issue_creates_ledger_consumptions_onhand_outbox_atomically() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    // Create item and receive 50 units at $10.00 each.
    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-ISS-001"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &make_receipt_req(&tenant_id, item.id, warehouse_id, 50, 1000, &format!("rcv-{}", Uuid::new_v4())),
    )
    .await
    .expect("receipt");

    // Issue 10 units.
    let issue_req = make_issue_req(&tenant_id, item.id, warehouse_id, 10, &format!("iss-{}", Uuid::new_v4()));
    let (result, is_replay) = process_issue(&pool, &issue_req).await.expect("issue");

    assert!(!is_replay, "first call must not be replay");
    assert_eq!(result.quantity, 10);
    assert_eq!(result.total_cost_minor, 10 * 1000);
    assert_eq!(result.consumed_layers.len(), 1);
    assert_eq!(result.consumed_layers[0].quantity, 10);
    assert_eq!(result.consumed_layers[0].extended_cost_minor, 10_000);

    // Verify ledger row (negative quantity for issue).
    let (entry_type, quantity): (String, i64) = sqlx::query_as(
        "SELECT entry_type::TEXT, quantity FROM inventory_ledger WHERE entry_id = $1",
    )
    .bind(result.issue_line_id)
    .fetch_one(&pool)
    .await
    .expect("ledger row");

    assert_eq!(entry_type, "issued");
    assert_eq!(quantity, -10, "ledger quantity must be negative for issues");

    // Verify layer_consumptions.
    let consumption_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM layer_consumptions WHERE ledger_entry_id = $1",
    )
    .bind(result.ledger_entry_id)
    .fetch_one(&pool)
    .await
    .expect("consumption count");
    assert_eq!(consumption_count, 1, "one consumption record per consumed layer");

    // Verify FIFO layer quantity_remaining = 50 - 10 = 40.
    let layer_remaining: i64 = sqlx::query_scalar(
        "SELECT quantity_remaining FROM inventory_layers WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("layer remaining");
    assert_eq!(layer_remaining, 40);

    // Verify on-hand projection updated.
    let (on_hand, total_cost): (i64, i64) = sqlx::query_as(
        "SELECT quantity_on_hand, total_cost_minor FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");
    assert_eq!(on_hand, 40, "quantity_on_hand = 50 - 10");
    assert_eq!(total_cost, 40 * 1000, "total_cost = remaining * unit_cost");

    // Verify outbox event.
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_id = $2",
    )
    .bind(&tenant_id)
    .bind(result.event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1, "exactly one outbox event");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: FIFO consumption across multiple layers (oldest first)
// ============================================================================

#[tokio::test]
#[serial]
async fn issue_consumes_fifo_layers_oldest_first() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-FIFO-001"))
        .await
        .expect("create item");

    // Layer 1: 5 units @ $10 (received first).
    process_receipt(
        &pool,
        &make_receipt_req(&tenant_id, item.id, warehouse_id, 5, 1000, &format!("rcv1-{}", Uuid::new_v4())),
    )
    .await
    .expect("receipt layer 1");

    // Layer 2: 10 units @ $20 (received second).
    process_receipt(
        &pool,
        &make_receipt_req(&tenant_id, item.id, warehouse_id, 10, 2000, &format!("rcv2-{}", Uuid::new_v4())),
    )
    .await
    .expect("receipt layer 2");

    // Issue 8 units: should consume all 5 from layer 1, then 3 from layer 2.
    let issue_req = make_issue_req(&tenant_id, item.id, warehouse_id, 8, &format!("iss-{}", Uuid::new_v4()));
    let (result, _) = process_issue(&pool, &issue_req).await.expect("issue 8 units");

    assert_eq!(result.quantity, 8);
    assert_eq!(result.consumed_layers.len(), 2, "two layers consumed");

    // Layer 1: 5 units @ $10 = $50
    assert_eq!(result.consumed_layers[0].quantity, 5);
    assert_eq!(result.consumed_layers[0].unit_cost_minor, 1000);
    assert_eq!(result.consumed_layers[0].extended_cost_minor, 5_000);

    // Layer 2: 3 units @ $20 = $60
    assert_eq!(result.consumed_layers[1].quantity, 3);
    assert_eq!(result.consumed_layers[1].unit_cost_minor, 2000);
    assert_eq!(result.consumed_layers[1].extended_cost_minor, 6_000);

    // Total cost = $50 + $60 = $110
    assert_eq!(result.total_cost_minor, 11_000);

    let sum: i64 = result.consumed_layers.iter().map(|c| c.extended_cost_minor).sum();
    assert_eq!(result.total_cost_minor, sum, "total_cost_minor must equal sum of extended costs");

    // Layer 1 should now be exhausted (quantity_remaining = 0).
    let remaining_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_layers WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND quantity_remaining > 0",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("remaining layers count");
    assert_eq!(remaining_count, 1, "only layer 2 should have remaining stock");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Idempotency — duplicate key returns stored result, no extra rows
// ============================================================================

#[tokio::test]
#[serial]
async fn issue_idempotency_replay_returns_stored_result() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-IDEM-ISS-001"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &make_receipt_req(&tenant_id, item.id, warehouse_id, 100, 500, &format!("rcv-{}", Uuid::new_v4())),
    )
    .await
    .expect("receipt");

    let idem_key = format!("iss-idem-{}", Uuid::new_v4());
    let req = make_issue_req(&tenant_id, item.id, warehouse_id, 10, &idem_key);

    // First call.
    let (r1, is_replay1) = process_issue(&pool, &req).await.expect("first issue");
    assert!(!is_replay1);

    // Second call — same key, same body.
    let (r2, is_replay2) = process_issue(&pool, &req).await.expect("second issue");
    assert!(is_replay2, "second call must be a replay");

    assert_eq!(r1.issue_line_id, r2.issue_line_id);
    assert_eq!(r1.event_id, r2.event_id);
    assert_eq!(r1.total_cost_minor, r2.total_cost_minor);

    // No duplicate ledger rows.
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND entry_type = 'issued'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("ledger count");
    assert_eq!(ledger_count, 1, "no duplicate ledger rows on replay");

    // Layer quantity_remaining not doubly decremented.
    let remaining: i64 = sqlx::query_scalar(
        "SELECT quantity_remaining FROM inventory_layers WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("layer remaining");
    assert_eq!(remaining, 90, "quantity_remaining must not be doubly decremented");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Guard — insufficient quantity rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn issue_guard_rejects_insufficient_quantity() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-INSUF-001"))
        .await
        .expect("create item");

    // Receive only 5 units.
    process_receipt(
        &pool,
        &make_receipt_req(&tenant_id, item.id, warehouse_id, 5, 1000, &format!("rcv-{}", Uuid::new_v4())),
    )
    .await
    .expect("receipt");

    // Attempt to issue 10 — exceeds available.
    let req = make_issue_req(&tenant_id, item.id, warehouse_id, 10, &format!("iss-{}", Uuid::new_v4()));
    let err = process_issue(&pool, &req).await.expect_err("must fail: insufficient stock");

    assert!(
        matches!(err, IssueError::InsufficientQuantity { requested: 10, available: 5 }),
        "expected InsufficientQuantity, got: {:?}", err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 5: Guard — inactive item rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn issue_guard_rejects_inactive_item() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-INACTIVE-ISS-001"))
        .await
        .expect("create item");

    ItemRepo::deactivate(&pool, item.id, &tenant_id)
        .await
        .expect("deactivate");

    let req = make_issue_req(&tenant_id, item.id, warehouse_id, 1, &format!("iss-{}", Uuid::new_v4()));
    let err = process_issue(&pool, &req).await.expect_err("inactive item must be rejected");

    assert!(
        matches!(err, IssueError::Guard(_)),
        "expected Guard error, got: {:?}", err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 6: Guard — zero quantity rejected (stateless)
// ============================================================================

#[tokio::test]
async fn issue_guard_rejects_zero_quantity() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let req = IssueRequest {
        quantity: 0,
        ..make_issue_req(&tenant_id, Uuid::new_v4(), Uuid::new_v4(), 1, &format!("iss-{}", Uuid::new_v4()))
    };

    let err = process_issue(&pool, &req).await.expect_err("zero qty must fail");
    assert!(matches!(err, IssueError::Guard(_)), "{:?}", err);
}

// ============================================================================
// Test 7: consumed_layers and source_ref carried in outbox event payload
// ============================================================================

#[tokio::test]
#[serial]
async fn issue_outbox_event_contains_consumed_layers_and_source_ref() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-OUTBOX-001"))
        .await
        .expect("create item");

    process_receipt(
        &pool,
        &make_receipt_req(&tenant_id, item.id, warehouse_id, 20, 1500, &format!("rcv-{}", Uuid::new_v4())),
    )
    .await
    .expect("receipt");

    let req = IssueRequest {
        source_module: "manufacturing".to_string(),
        source_type: "work_order".to_string(),
        source_id: "WO-42".to_string(),
        source_line_id: Some("WO-42-L1".to_string()),
        ..make_issue_req(&tenant_id, item.id, warehouse_id, 5, &format!("iss-{}", Uuid::new_v4()))
    };

    let (result, _) = process_issue(&pool, &req).await.expect("issue");

    // Read outbox payload and verify structure.
    let payload_json: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE event_id = $1",
    )
    .bind(result.event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox payload");

    // consumed_layers must be present and non-empty
    let consumed_layers = payload_json["payload"]["consumed_layers"]
        .as_array()
        .expect("consumed_layers must be an array");
    assert!(!consumed_layers.is_empty(), "consumed_layers must be non-empty");

    // extended_cost_minor present in each layer
    for layer in consumed_layers {
        assert!(layer.get("extended_cost_minor").is_some(), "each layer needs extended_cost_minor");
        assert!(layer.get("layer_id").is_some());
    }

    // source_ref must be present with correct values
    let source_ref = &payload_json["payload"]["source_ref"];
    assert_eq!(source_ref["source_module"], "manufacturing");
    assert_eq!(source_ref["source_type"], "work_order");
    assert_eq!(source_ref["source_id"], "WO-42");
    assert_eq!(source_ref["source_line_id"], "WO-42-L1");

    // total_cost_minor must equal sum of extended costs
    let total: i64 = consumed_layers
        .iter()
        .map(|l| l["extended_cost_minor"].as_i64().unwrap_or(0))
        .sum();
    assert_eq!(
        payload_json["payload"]["total_cost_minor"].as_i64().unwrap(),
        total,
        "total_cost_minor must equal sum of extended_cost_minor"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}
