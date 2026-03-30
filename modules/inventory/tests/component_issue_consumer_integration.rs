//! Integration tests for the component issue consumer.
//!
//! Tests run against a real PostgreSQL database (inventory_db).
//! Verifies that `production.component_issue.requested` events are correctly
//! translated into Inventory stock issues with FIFO consumption and
//! proper source_ref linking back to the work order.

use inventory_rs::consumers::component_issue_consumer::{
    process_component_issue_request, ComponentIssueItem, ComponentIssueRequestedPayload,
    ConsumerError,
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
        name: "Component Issue Test Item".to_string(),
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
        correlation_id: Some("ci-test-stock".to_string()),
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
// 1. Happy path: single item component issue
// ============================================================================

#[tokio::test]
#[serial]
async fn component_issue_single_item_happy_path() {
    let pool = setup_db().await;
    let tenant = format!("ci-single-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    // Create item and stock it
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("CI-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    stock_item(&pool, &tenant, item.id, warehouse_id, 100, 500).await;

    // Process component issue request
    let event_id = Uuid::new_v4();
    let payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-TEST-001".to_string(),
        items: vec![ComponentIssueItem {
            item_id: item.id,
            warehouse_id,
            quantity: 10,
            currency: "usd".to_string(),
        }],
    };

    let results =
        process_component_issue_request(&pool, event_id, &payload, Some("ci-test-corr"), None)
            .await
            .expect("process component issue");

    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.quantity, 10);
    assert_eq!(result.total_cost_minor, 5000); // 10 * 500
    assert_eq!(result.source_ref.source_module, "production");
    assert_eq!(result.source_ref.source_type, "production");
    assert_eq!(result.source_ref.source_id, wo_id.to_string());
    assert_eq!(
        result.source_ref.source_line_id.as_deref(),
        Some("WO-TEST-001")
    );

    // Verify ledger entry
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'issued'",
    )
    .bind(&tenant)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("fetch ledger count");
    assert_eq!(ledger_count, 1);

    // Verify outbox event with production source_ref
    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_issued' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch outbox");

    let src = &outbox_payload["payload"]["source_ref"];
    assert_eq!(src["source_module"].as_str(), Some("production"));
    assert_eq!(src["source_type"].as_str(), Some("production"));
    assert_eq!(src["source_id"].as_str(), Some(wo_id.to_string().as_str()));
}

// ============================================================================
// 2. Multiple items in one request
// ============================================================================

#[tokio::test]
#[serial]
async fn component_issue_multiple_items() {
    let pool = setup_db().await;
    let tenant = format!("ci-multi-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    let item_a = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("CIA-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item A");

    let item_b = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("CIB-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item B");

    stock_item(&pool, &tenant, item_a.id, wh, 50, 200).await;
    stock_item(&pool, &tenant, item_b.id, wh, 30, 1000).await;

    let event_id = Uuid::new_v4();
    let payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-MULTI-001".to_string(),
        items: vec![
            ComponentIssueItem {
                item_id: item_a.id,
                warehouse_id: wh,
                quantity: 5,
                currency: "usd".to_string(),
            },
            ComponentIssueItem {
                item_id: item_b.id,
                warehouse_id: wh,
                quantity: 3,
                currency: "usd".to_string(),
            },
        ],
    };

    let results =
        process_component_issue_request(&pool, event_id, &payload, Some("ci-multi-corr"), None)
            .await
            .expect("process multi-item issue");

    assert_eq!(results.len(), 2);

    // Item A: 5 * 200 = 1000
    assert_eq!(results[0].item_id, item_a.id);
    assert_eq!(results[0].quantity, 5);
    assert_eq!(results[0].total_cost_minor, 1000);

    // Item B: 3 * 1000 = 3000
    assert_eq!(results[1].item_id, item_b.id);
    assert_eq!(results[1].quantity, 3);
    assert_eq!(results[1].total_cost_minor, 3000);
}

// ============================================================================
// 3. FIFO consumption verified
// ============================================================================

#[tokio::test]
#[serial]
async fn component_issue_fifo_consumption() {
    let pool = setup_db().await;
    let tenant = format!("ci-fifo-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("FIFO-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    // Stock two layers at different costs
    stock_item(&pool, &tenant, item.id, wh, 10, 100).await; // layer 1: 10 @ $1.00
    stock_item(&pool, &tenant, item.id, wh, 10, 200).await; // layer 2: 10 @ $2.00

    let event_id = Uuid::new_v4();
    let payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-FIFO-001".to_string(),
        items: vec![ComponentIssueItem {
            item_id: item.id,
            warehouse_id: wh,
            quantity: 15, // spans both layers
            currency: "usd".to_string(),
        }],
    };

    let results =
        process_component_issue_request(&pool, event_id, &payload, Some("ci-fifo-corr"), None)
            .await
            .expect("process fifo issue");

    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.quantity, 15);

    // FIFO: 10 * 100 + 5 * 200 = 1000 + 1000 = 2000
    assert_eq!(result.total_cost_minor, 2000);
    assert_eq!(result.consumed_layers.len(), 2);
}

// ============================================================================
// 4. Idempotency: same event processed twice
// ============================================================================

#[tokio::test]
#[serial]
async fn component_issue_idempotent() {
    let pool = setup_db().await;
    let tenant = format!("ci-idem-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("IDEM-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    stock_item(&pool, &tenant, item.id, wh, 100, 500).await;

    let event_id = Uuid::new_v4();
    let payload = ComponentIssueRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-IDEM-001".to_string(),
        items: vec![ComponentIssueItem {
            item_id: item.id,
            warehouse_id: wh,
            quantity: 5,
            currency: "usd".to_string(),
        }],
    };

    // First call
    let r1 = process_component_issue_request(&pool, event_id, &payload, Some("ci-idem-corr"), None)
        .await
        .expect("first call");

    // Second call with same event_id — should replay
    let r2 = process_component_issue_request(&pool, event_id, &payload, Some("ci-idem-corr"), None)
        .await
        .expect("second call (replay)");

    assert_eq!(r1[0].issue_line_id, r2[0].issue_line_id);
    assert_eq!(r1[0].total_cost_minor, r2[0].total_cost_minor);

    // Only one ledger entry should exist
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'issued'",
    )
    .bind(&tenant)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("fetch ledger count");
    assert_eq!(ledger_count, 1);
}

// ============================================================================
// 5. Insufficient stock fails
// ============================================================================

#[tokio::test]
#[serial]
async fn component_issue_insufficient_stock() {
    let pool = setup_db().await;
    let tenant = format!("ci-insuf-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("INS-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    stock_item(&pool, &tenant, item.id, wh, 5, 500).await;

    let event_id = Uuid::new_v4();
    let payload = ComponentIssueRequestedPayload {
        work_order_id: Uuid::new_v4(),
        tenant_id: tenant.clone(),
        order_number: "WO-INS-001".to_string(),
        items: vec![ComponentIssueItem {
            item_id: item.id,
            warehouse_id: wh,
            quantity: 10, // more than available
            currency: "usd".to_string(),
        }],
    };

    let err = process_component_issue_request(&pool, event_id, &payload, None, None)
        .await
        .expect_err("should fail on insufficient stock");

    assert!(
        matches!(err, ConsumerError::Issue(_)),
        "Expected Issue error, got: {:?}",
        err
    );
}

// ============================================================================
// 6. Empty items list rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn component_issue_empty_items_rejected() {
    let pool = setup_db().await;

    let payload = ComponentIssueRequestedPayload {
        work_order_id: Uuid::new_v4(),
        tenant_id: "any-tenant".to_string(),
        order_number: "WO-EMPTY".to_string(),
        items: vec![],
    };

    let err = process_component_issue_request(&pool, Uuid::new_v4(), &payload, None, None)
        .await
        .expect_err("should reject empty items");

    assert!(
        matches!(err, ConsumerError::Validation(_)),
        "Expected Validation, got: {:?}",
        err
    );
}
