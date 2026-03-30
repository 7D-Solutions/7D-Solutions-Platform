//! Integration tests for the FG receipt consumer.
//!
//! Tests run against a real PostgreSQL database (inventory_db).
//! Verifies the end-to-end flow:
//!   1. Stock components via receipts
//!   2. Issue components (simulating production consumption)
//!   3. Request FG receipt
//!   4. Assert receipt unit_cost = sum(component FIFO costs) / fg_qty

use inventory_rs::consumers::component_issue_consumer::{
    process_component_issue_request, ComponentIssueItem, ComponentIssueRequestedPayload,
};
use inventory_rs::consumers::fg_receipt_consumer::{
    process_fg_receipt_request, FgReceiptRequestedPayload,
};
use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
    });

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
        name: "FG Receipt Test Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

async fn stock_item(
    pool: &sqlx::PgPool,
    tenant: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    cost: i64,
) {
    let req = ReceiptRequest {
        tenant_id: tenant.to_string(),
        item_id,
        warehouse_id,
        quantity: qty,
        unit_cost_minor: cost,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("stock-{}", Uuid::new_v4()),
        correlation_id: Some("fg-test-stock".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    };
    process_receipt(pool, &req, None)
        .await
        .expect("stock item via receipt");
}

// ============================================================================
// 1. Happy path: single component → FG receipt with correct rolled-up cost
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_single_component_cost_rollup() {
    let pool = setup_db().await;
    let tenant = format!("fg-single-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    // Create component item and FG item
    let component = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("COMP-{}", Uuid::new_v4())),
    )
    .await
    .expect("create component");

    let fg_item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("FG-{}", Uuid::new_v4())),
    )
    .await
    .expect("create FG item");

    // Stock component: 100 units @ $5.00 each (500 minor units)
    stock_item(&pool, &tenant, component.id, warehouse_id, 100, 500).await;

    // Issue 10 components for work order (simulating production consumption)
    let issue_event_id = Uuid::new_v4();
    let issue_payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-001".to_string(),
        items: vec![ComponentIssueItem {
            item_id: component.id,
            warehouse_id,
            quantity: 10,
            currency: "usd".to_string(),
        }],
    };

    let issue_results = process_component_issue_request(
        &pool,
        issue_event_id,
        &issue_payload,
        Some("fg-test-corr"),
        None,
    )
    .await
    .expect("issue components");

    assert_eq!(issue_results.len(), 1);
    let total_component_cost = issue_results[0].total_cost_minor;
    assert_eq!(total_component_cost, 5000); // 10 * 500

    // Request FG receipt: 5 finished goods
    let fg_qty: i64 = 5;
    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-001".to_string(),
        item_id: fg_item.id,
        warehouse_id,
        quantity: fg_qty,
        currency: "usd".to_string(),
    };

    let result =
        process_fg_receipt_request(&pool, fg_event_id, &fg_payload, Some("fg-test-corr"), None)
            .await
            .expect("process FG receipt");

    // Arithmetic spot-check:
    // Expected unit cost = 5000 / 5 = 1000 (= $10.00 per FG unit)
    let expected_unit_cost = total_component_cost / fg_qty;
    assert_eq!(result.unit_cost_minor, expected_unit_cost);
    assert_eq!(result.unit_cost_minor, 1000);
    assert_eq!(result.source_type, "production");
    assert_eq!(result.quantity, fg_qty);

    // Verify: sum(component FIFO costs) == total receipt extended cost
    let receipt_extended_cost = result.unit_cost_minor * result.quantity;
    assert_eq!(receipt_extended_cost, total_component_cost);
}

// ============================================================================
// 2. Multiple components with different FIFO layers
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_multiple_components_fifo_cost_rollup() {
    let pool = setup_db().await;
    let tenant = format!("fg-multi-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    // Create two component items and one FG item
    let comp_a = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("CA-{}", Uuid::new_v4())),
    )
    .await
    .expect("create component A");

    let comp_b = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("CB-{}", Uuid::new_v4())),
    )
    .await
    .expect("create component B");

    let fg_item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("FG-{}", Uuid::new_v4())),
    )
    .await
    .expect("create FG item");

    // Stock component A: 50 @ $2.00 (200 minor)
    stock_item(&pool, &tenant, comp_a.id, warehouse_id, 50, 200).await;

    // Stock component B with two FIFO layers:
    //   Layer 1: 10 @ $10.00 (1000 minor)
    //   Layer 2: 20 @ $15.00 (1500 minor)
    stock_item(&pool, &tenant, comp_b.id, warehouse_id, 10, 1000).await;
    stock_item(&pool, &tenant, comp_b.id, warehouse_id, 20, 1500).await;

    // Issue components for the work order:
    //   Component A: 20 units → 20 * 200 = 4000
    //   Component B: 15 units → FIFO: 10*1000 + 5*1500 = 10000 + 7500 = 17500
    let issue_event_id = Uuid::new_v4();
    let issue_payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-MULTI".to_string(),
        items: vec![
            ComponentIssueItem {
                item_id: comp_a.id,
                warehouse_id,
                quantity: 20,
                currency: "usd".to_string(),
            },
            ComponentIssueItem {
                item_id: comp_b.id,
                warehouse_id,
                quantity: 15,
                currency: "usd".to_string(),
            },
        ],
    };

    let issue_results = process_component_issue_request(
        &pool,
        issue_event_id,
        &issue_payload,
        Some("fg-multi-corr"),
        None,
    )
    .await
    .expect("issue components");

    assert_eq!(issue_results.len(), 2);
    let cost_a = issue_results[0].total_cost_minor; // 4000
    let cost_b = issue_results[1].total_cost_minor; // 17500
    assert_eq!(cost_a, 4000);
    assert_eq!(cost_b, 17500);
    let total_component_cost = cost_a + cost_b; // 21500

    // Request FG receipt: 10 finished goods
    let fg_qty: i64 = 10;
    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-MULTI".to_string(),
        item_id: fg_item.id,
        warehouse_id,
        quantity: fg_qty,
        currency: "usd".to_string(),
    };

    let result =
        process_fg_receipt_request(&pool, fg_event_id, &fg_payload, Some("fg-multi-corr"), None)
            .await
            .expect("process FG receipt");

    // Arithmetic spot-check:
    // Expected unit cost = 21500 / 10 = 2150 (= $21.50 per FG unit)
    let expected_unit_cost = total_component_cost / fg_qty;
    assert_eq!(result.unit_cost_minor, expected_unit_cost);
    assert_eq!(result.unit_cost_minor, 2150);
    assert_eq!(result.source_type, "production");

    // Invariant: sum(component FIFO costs) >= total receipt extended cost
    // (integer division may truncate, so >= not ==)
    let receipt_extended_cost = result.unit_cost_minor * result.quantity;
    assert!(
        total_component_cost >= receipt_extended_cost,
        "Component cost {} must be >= receipt extended cost {}",
        total_component_cost,
        receipt_extended_cost
    );
}

// ============================================================================
// 3. No component issues → rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_no_components_rejected() {
    let pool = setup_db().await;
    let tenant = format!("fg-nocomp-{}", Uuid::new_v4());
    let wo_id = Uuid::new_v4();

    let fg_item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("FG-{}", Uuid::new_v4())),
    )
    .await
    .expect("create FG item");

    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-NOCOMP".to_string(),
        item_id: fg_item.id,
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        currency: "usd".to_string(),
    };

    let err = process_fg_receipt_request(&pool, fg_event_id, &fg_payload, None, None)
        .await
        .expect_err("should reject FG receipt with no component issues");

    assert!(
        matches!(
            err,
            inventory_rs::consumers::fg_receipt_consumer::ConsumerError::Validation(_)
        ),
        "Expected Validation error, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Idempotency: same FG receipt processed twice
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_idempotent() {
    let pool = setup_db().await;
    let tenant = format!("fg-idem-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    let component = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("COMP-{}", Uuid::new_v4())),
    )
    .await
    .expect("create component");

    let fg_item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("FG-{}", Uuid::new_v4())),
    )
    .await
    .expect("create FG item");

    stock_item(&pool, &tenant, component.id, warehouse_id, 50, 300).await;

    // Issue 10 components
    let issue_event_id = Uuid::new_v4();
    let issue_payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-IDEM".to_string(),
        items: vec![ComponentIssueItem {
            item_id: component.id,
            warehouse_id,
            quantity: 10,
            currency: "usd".to_string(),
        }],
    };

    process_component_issue_request(
        &pool,
        issue_event_id,
        &issue_payload,
        Some("fg-idem-corr"),
        None,
    )
    .await
    .expect("issue components");

    // Request FG receipt twice with same event_id
    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-IDEM".to_string(),
        item_id: fg_item.id,
        warehouse_id,
        quantity: 5,
        currency: "usd".to_string(),
    };

    let r1 =
        process_fg_receipt_request(&pool, fg_event_id, &fg_payload, Some("fg-idem-corr"), None)
            .await
            .expect("first FG receipt");

    let r2 =
        process_fg_receipt_request(&pool, fg_event_id, &fg_payload, Some("fg-idem-corr"), None)
            .await
            .expect("second FG receipt (replay)");

    assert_eq!(r1.receipt_line_id, r2.receipt_line_id);
    assert_eq!(r1.unit_cost_minor, r2.unit_cost_minor);

    // Only one ledger received entry should exist for FG
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'received' AND source_type = 'production'",
    )
    .bind(&tenant)
    .bind(fg_item.id)
    .fetch_one(&pool)
    .await
    .expect("fetch ledger count");
    assert_eq!(ledger_count, 1);
}

// ============================================================================
// 5. Zero quantity rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_zero_quantity_rejected() {
    let pool = setup_db().await;

    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: Uuid::new_v4(),
        tenant_id: "any-tenant".to_string(),
        order_number: "WO-ZERO".to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 0,
        currency: "usd".to_string(),
    };

    let err = process_fg_receipt_request(&pool, Uuid::new_v4(), &fg_payload, None, None)
        .await
        .expect_err("should reject zero quantity");

    assert!(
        matches!(
            err,
            inventory_rs::consumers::fg_receipt_consumer::ConsumerError::Validation(_)
        ),
        "Expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// 6. Outbox event emitted with source_type=production
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_emits_item_received_event() {
    let pool = setup_db().await;
    let tenant = format!("fg-event-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    let component = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("COMP-{}", Uuid::new_v4())),
    )
    .await
    .expect("create component");

    let fg_item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("FG-{}", Uuid::new_v4())),
    )
    .await
    .expect("create FG item");

    stock_item(&pool, &tenant, component.id, warehouse_id, 100, 800).await;

    // Issue 20 components → 20 * 800 = 16000 total cost
    let issue_event_id = Uuid::new_v4();
    let issue_payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-EVT".to_string(),
        items: vec![ComponentIssueItem {
            item_id: component.id,
            warehouse_id,
            quantity: 20,
            currency: "usd".to_string(),
        }],
    };

    process_component_issue_request(
        &pool,
        issue_event_id,
        &issue_payload,
        Some("fg-evt-corr"),
        None,
    )
    .await
    .expect("issue components");

    // FG receipt: 4 units → 16000/4 = 4000 per unit
    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FG-EVT".to_string(),
        item_id: fg_item.id,
        warehouse_id,
        quantity: 4,
        currency: "usd".to_string(),
    };

    process_fg_receipt_request(&pool, fg_event_id, &fg_payload, Some("fg-evt-corr"), None)
        .await
        .expect("process FG receipt");

    // Verify outbox has inventory.item_received with source_type=production
    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_received' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch outbox");

    let p = &outbox_payload["payload"];
    assert_eq!(p["source_type"].as_str(), Some("production"));
    assert_eq!(p["unit_cost_minor"].as_i64(), Some(4000));
    assert_eq!(p["quantity"].as_i64(), Some(4));
    assert_eq!(p["item_id"].as_str(), Some(fg_item.id.to_string().as_str()));
}
