//! Integration tests for movement history query (bd-16vh).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL=postgres://inventory_user:inventory_pass@localhost:5442/inventory_db
//!
//! Coverage:
//! 1. history_empty_for_new_item          — no movements returns empty list
//! 2. history_ordered_after_receipts      — two receipts in posted_at order, ledger id tie-break
//! 3. history_location_filter             — location_id=Some filters to that location only
//! 4. history_location_none_all_movements — location_id=None returns all movements

use inventory_rs::domain::{
    history::query::query_movement_history,
    items::{CreateItemRequest, ItemRepo, TrackingMode},
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

fn make_item(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "History Test Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    }
}

fn make_receipt(tenant_id: &str, item_id: Uuid, warehouse_id: Uuid, qty: i64) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        unit_cost_minor: 1000,
        currency: "usd".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("test-corr".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
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
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM layer_consumptions WHERE tenant_id = $1")
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
async fn history_empty_for_new_item() {
    let pool = setup_db().await;
    let tenant_id = format!("e2e-hist-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant_id, "HIST-EMPTY-001"))
        .await
        .expect("create item");

    let movements = query_movement_history(&pool, &tenant_id, item.id, None)
        .await
        .expect("query");

    assert!(movements.is_empty(), "new item must have no movements");

    cleanup(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn history_ordered_after_receipts() {
    let pool = setup_db().await;
    let tenant_id = format!("e2e-hist-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant_id, "HIST-ORDER-001"))
        .await
        .expect("create item");

    // Post two receipts; expect both in ledger-id order.
    let r1 = make_receipt(&tenant_id, item.id, warehouse_id, 100);
    let r2 = make_receipt(&tenant_id, item.id, warehouse_id, 50);
    process_receipt(&pool, &r1, None).await.expect("receipt 1");
    process_receipt(&pool, &r2, None).await.expect("receipt 2");

    let movements = query_movement_history(&pool, &tenant_id, item.id, None)
        .await
        .expect("query");

    assert_eq!(movements.len(), 2, "must have exactly 2 movements");
    // Ordered by posted_at ASC, id ASC — first receipt was inserted first.
    assert!(
        movements[0].ledger_id < movements[1].ledger_id,
        "must be in ledger-id order"
    );
    assert_eq!(movements[0].entry_type, "received");
    assert_eq!(movements[0].quantity, 100);
    assert_eq!(movements[1].quantity, 50);
    // source_ref fields present
    assert_eq!(
        movements[0].reference_type.as_deref(),
        Some("purchase_order")
    );

    cleanup(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn history_location_filter() {
    let pool = setup_db().await;
    let tenant_id = format!("e2e-hist-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant_id, "HIST-LOC-001"))
        .await
        .expect("create item");

    // Insert a location row directly so we have a valid location_id FK.
    let location_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO locations (id, tenant_id, warehouse_id, code, name, created_at, updated_at)
           VALUES ($1, $2, $3, 'BIN-A1', 'Bin A1', NOW(), NOW())"#,
    )
    .bind(location_id)
    .bind(&tenant_id)
    .bind(warehouse_id)
    .execute(&pool)
    .await
    .expect("insert location");

    // Receipt without location (NULL location_id).
    let r_no_loc = make_receipt(&tenant_id, item.id, warehouse_id, 200);
    process_receipt(&pool, &r_no_loc, None)
        .await
        .expect("receipt no-loc");

    // Receipt with location — patch the ledger row directly after insert (simulating
    // a location-aware receipt).  We call the receipt service then update location_id
    // since the integration-test ReceiptRequest doesn't carry location_id yet.
    let r_loc = make_receipt(&tenant_id, item.id, warehouse_id, 75);
    process_receipt(&pool, &r_loc, None)
        .await
        .expect("receipt with-loc");

    // Patch most recent ledger row for this tenant to have location_id.
    sqlx::query(
        "UPDATE inventory_ledger SET location_id = $1 WHERE tenant_id = $2 AND item_id = $3
         AND id = (SELECT MAX(id) FROM inventory_ledger WHERE tenant_id = $2 AND item_id = $3)",
    )
    .bind(location_id)
    .bind(&tenant_id)
    .bind(item.id)
    .execute(&pool)
    .await
    .expect("patch location_id");

    // Filter by location_id → only the patched row.
    let loc_movements = query_movement_history(&pool, &tenant_id, item.id, Some(location_id))
        .await
        .expect("query filtered");

    assert_eq!(loc_movements.len(), 1, "only one movement at this location");
    assert_eq!(loc_movements[0].quantity, 75);
    assert_eq!(loc_movements[0].location_id, Some(location_id));

    cleanup(&pool, &tenant_id).await;
    sqlx::query("DELETE FROM locations WHERE id = $1")
        .bind(location_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn history_location_none_all_movements() {
    let pool = setup_db().await;
    let tenant_id = format!("e2e-hist-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item(&tenant_id, "HIST-ALL-001"))
        .await
        .expect("create item");

    // Three receipts
    for qty in [10, 20, 30] {
        let r = make_receipt(&tenant_id, item.id, warehouse_id, qty);
        process_receipt(&pool, &r, None).await.expect("receipt");
    }

    let all_movements = query_movement_history(&pool, &tenant_id, item.id, None)
        .await
        .expect("query all");

    assert_eq!(all_movements.len(), 3, "must return all 3 movements");

    // Confirm deterministic ordering: ledger ids strictly ascending.
    let ids: Vec<i64> = all_movements.iter().map(|m| m.ledger_id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "movements must be in ledger-id order");

    cleanup(&pool, &tenant_id).await;
}
