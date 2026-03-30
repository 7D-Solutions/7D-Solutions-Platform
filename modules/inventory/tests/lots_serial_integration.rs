//! Integration tests for lot/serial query + trace endpoints (bd-3w2o).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. list_lots_for_item returns lots for a lot-tracked item
//! 2. list_serials_for_item returns serial instances for a serial-tracked item
//! 3. trace_lot returns movement history for a lot (receipt + issue movements)
//! 4. trace_serial returns movement history for a serial (receipt + issue)
//! 5. Empty results for item with no lots/serials (not an error)
//! 6. Tenant isolation: lots from other tenants are excluded

use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    lots_serials::queries::{list_lots_for_item, list_serials_for_item, trace_lot, trace_serial},
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
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require"
            .to_string()
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

async fn create_lot_item(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    ItemRepo::create(
        pool,
        &CreateItemRequest {
            tenant_id: tenant_id.to_string(),
            sku: format!("LOT-{}", Uuid::new_v4()),
            name: "Lot Widget".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::Lot,
            make_buy: None,
        },
    )
    .await
    .expect("create lot item")
    .id
}

async fn create_serial_item(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    ItemRepo::create(
        pool,
        &CreateItemRequest {
            tenant_id: tenant_id.to_string(),
            sku: format!("SER-{}", Uuid::new_v4()),
            name: "Serial Widget".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::Serial,
            make_buy: None,
        },
    )
    .await
    .expect("create serial item")
    .id
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM layer_consumptions WHERE EXISTS (SELECT 1 FROM inventory_layers il WHERE il.id = layer_id AND il.tenant_id = $1)")
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
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_serial_instances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_lots WHERE tenant_id = $1")
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
async fn test_list_lots_returns_lots_for_item() {
    let pool = setup_db().await;
    let tenant = format!("t-lots-list-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;

    // Receive with lot A
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 10,
            unit_cost_minor: 500,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-lots-a".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-A".to_string()),
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt lot A");

    // Receive with lot B
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 5,
            unit_cost_minor: 600,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-lots-b".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-B".to_string()),
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt lot B");

    let lots = list_lots_for_item(&pool, &tenant, item_id)
        .await
        .expect("list lots");

    assert_eq!(lots.len(), 2, "should have 2 lots");
    let codes: Vec<&str> = lots.iter().map(|l| l.lot_code.as_str()).collect();
    assert!(codes.contains(&"LOT-A"), "LOT-A expected");
    assert!(codes.contains(&"LOT-B"), "LOT-B expected");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_list_serials_returns_instances_for_item() {
    let pool = setup_db().await;
    let tenant = format!("t-ser-list-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_serial_item(&pool, &tenant).await;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 2,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-ser-1".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: Some(vec!["SN-001".to_string(), "SN-002".to_string()]),
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt serials");

    let serials = list_serials_for_item(&pool, &tenant, item_id)
        .await
        .expect("list serials");

    assert_eq!(serials.len(), 2, "should have 2 serial instances");
    let codes: Vec<&str> = serials.iter().map(|s| s.serial_code.as_str()).collect();
    assert!(codes.contains(&"SN-001"));
    assert!(codes.contains(&"SN-002"));

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_trace_lot_returns_receipt_and_issue_movements() {
    let pool = setup_db().await;
    let tenant = format!("t-trace-lot-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 20,
            unit_cost_minor: 300,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-trace-lot".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("TRACE-LOT".to_string()),
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 5,
            currency: "usd".to_string(),
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-001".to_string(),
            source_line_id: None,
            idempotency_key: "issue-trace-lot".to_string(),
            correlation_id: None,
            causation_id: None,
            location_id: None,
            uom_id: None,
            lot_code: Some("TRACE-LOT".to_string()),
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("issue");

    let movements = trace_lot(&pool, &tenant, item_id, "TRACE-LOT")
        .await
        .expect("trace lot");

    assert_eq!(movements.len(), 2, "receipt + issue = 2 movements");

    let types: Vec<&str> = movements.iter().map(|m| m.entry_type.as_str()).collect();
    assert!(types.contains(&"received"), "received entry expected");
    assert!(types.contains(&"issued"), "issued entry expected");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_trace_serial_returns_receipt_and_issue_movements() {
    let pool = setup_db().await;
    let tenant = format!("t-trace-ser-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_serial_item(&pool, &tenant).await;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 1,
            unit_cost_minor: 2000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-trace-ser".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: Some(vec!["SN-TRACE".to_string()]),
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 1,
            currency: "usd".to_string(),
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-002".to_string(),
            source_line_id: None,
            idempotency_key: "issue-trace-ser".to_string(),
            correlation_id: None,
            causation_id: None,
            location_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: Some(vec!["SN-TRACE".to_string()]),
        },
        None,
    )
    .await
    .expect("issue");

    let movements = trace_serial(&pool, &tenant, item_id, "SN-TRACE")
        .await
        .expect("trace serial");

    assert_eq!(movements.len(), 2, "receipt + issue = 2 movements");

    let types: Vec<&str> = movements.iter().map(|m| m.entry_type.as_str()).collect();
    assert!(types.contains(&"received"));
    assert!(types.contains(&"issued"));

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_empty_results_for_item_with_no_lots() {
    let pool = setup_db().await;
    let tenant = format!("t-empty-lots-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;

    let lots = list_lots_for_item(&pool, &tenant, item_id)
        .await
        .expect("list lots");
    assert!(lots.is_empty(), "no lots yet = empty vec");

    let trace = trace_lot(&pool, &tenant, item_id, "NONEXISTENT")
        .await
        .expect("trace lot");
    assert!(trace.is_empty(), "unknown lot = empty vec");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_tenant_isolation_lots() {
    let pool = setup_db().await;
    let tenant_a = format!("t-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("t-iso-b-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;

    let item_a = create_lot_item(&pool, &tenant_a).await;
    let item_b = create_lot_item(&pool, &tenant_b).await;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant_a.clone(),
            item_id: item_a,
            warehouse_id: wh,
            quantity: 5,
            unit_cost_minor: 100,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-iso-a".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("SHARED-CODE".to_string()),
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt tenant A");

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant_b.clone(),
            item_id: item_b,
            warehouse_id: wh,
            quantity: 3,
            unit_cost_minor: 200,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-iso-b".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("SHARED-CODE".to_string()),
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt tenant B");

    // tenant_a's lots for item_a = only their own
    let lots_a = list_lots_for_item(&pool, &tenant_a, item_a)
        .await
        .expect("list lots A");
    assert_eq!(lots_a.len(), 1, "tenant A has exactly 1 lot");
    assert_eq!(lots_a[0].tenant_id, tenant_a);

    // tenant_b's lots for item_b = only their own
    let lots_b = list_lots_for_item(&pool, &tenant_b, item_b)
        .await
        .expect("list lots B");
    assert_eq!(lots_b.len(), 1, "tenant B has exactly 1 lot");
    assert_eq!(lots_b[0].tenant_id, tenant_b);

    // Cross-tenant: tenant_a querying item_b returns nothing
    let cross = list_lots_for_item(&pool, &tenant_a, item_b)
        .await
        .expect("cross tenant query");
    assert!(cross.is_empty(), "tenant A cannot see tenant B lots");

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}
