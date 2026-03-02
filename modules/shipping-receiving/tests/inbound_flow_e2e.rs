//! E2E integration tests: Inbound shipment workflow (bd-2imv).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. Inbound happy path: draft → confirmed → in_transit → arrived → receiving → closed
//! 2. Invariant failure: close with qty mismatch rejects, then fix and close succeeds
//! 3. Inventory receipt linkage on close
//! 4. Outbox events emitted for create and each status transition

use chrono::Utc;
use serial_test::serial;
use shipping_receiving_rs::{
    domain::shipments::{ShipmentService, TransitionRequest},
    InventoryIntegration,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── Test DB helpers ─────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB");

    let _ = sqlx::migrate!("db/migrations").run(&pool).await;
    pool
}

async fn insert_shipment(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    direction: &str,
    status: &str,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(tenant_id)
    .bind(direction)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert shipment");
    row.0
}

#[allow(clippy::too_many_arguments)]
async fn insert_line_full(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    qty_expected: i64,
    qty_accepted: i64,
    qty_rejected: i64,
    qty_received: i64,
    warehouse_id: Option<Uuid>,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_accepted,
             qty_rejected, qty_received, warehouse_id)
        VALUES ($1, $2, 'TEST-SKU', $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(qty_expected)
    .bind(qty_accepted)
    .bind(qty_rejected)
    .bind(qty_received)
    .bind(warehouse_id)
    .fetch_one(pool)
    .await
    .expect("insert line full");
    row.0
}

async fn get_shipment_status(pool: &sqlx::PgPool, id: Uuid) -> String {
    let row: (String,) = sqlx::query_as("SELECT status FROM shipments WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("get shipment status");
    row.0
}

async fn get_inventory_ref(pool: &sqlx::PgPool, line_id: Uuid) -> Option<Uuid> {
    let row: (Option<Uuid>,) =
        sqlx::query_as("SELECT inventory_ref_id FROM shipment_lines WHERE id = $1")
            .bind(line_id)
            .fetch_one(pool)
            .await
            .expect("get inventory_ref_id");
    row.0
}

async fn count_outbox_events(pool: &sqlx::PgPool, aggregate_id: &str, event_type: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sr_events_outbox WHERE aggregate_id = $1 AND event_type = $2",
    )
    .bind(aggregate_id)
    .bind(event_type)
    .fetch_one(pool)
    .await
    .expect("count outbox events");
    row.0
}

fn make_transition(status: &str) -> TransitionRequest {
    TransitionRequest {
        status: status.to_string(),
        arrived_at: if status == "arrived" {
            Some(Utc::now())
        } else {
            None
        },
        shipped_at: None,
        delivered_at: None,
        closed_at: if status == "closed" {
            Some(Utc::now())
        } else {
            None
        },
    }
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn inbound_happy_path_draft_to_closed() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    // Create inbound shipment in draft
    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "draft").await;

    // Add a line (expected=10)
    let line_id = insert_line_full(&pool, tenant_id, ship_id, 10, 0, 0, 0, Some(wh)).await;

    // Progress: draft → confirmed → in_transit → arrived → receiving
    for status in &["confirmed", "in_transit", "arrived", "receiving"] {
        let req = make_transition(status);
        let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
        assert!(
            result.is_ok(),
            "transition to {status} failed: {:?}",
            result.err()
        );
    }

    assert_eq!(get_shipment_status(&pool, ship_id).await, "receiving");

    // Record receipt on line: received=10, accepted=10, rejected=0
    sqlx::query(
        "UPDATE shipment_lines SET qty_received = 10, qty_accepted = 10, qty_rejected = 0 WHERE id = $1",
    )
    .bind(line_id)
    .execute(&pool)
    .await
    .expect("update line qtys");

    // Close
    let close_req = make_transition("closed");
    let result =
        ShipmentService::transition(&pool, ship_id, tenant_id, &close_req, &inventory).await;
    assert!(result.is_ok(), "close failed: {:?}", result.err());
    assert_eq!(get_shipment_status(&pool, ship_id).await, "closed");

    // Verify inventory receipt linkage
    let ref_id = get_inventory_ref(&pool, line_id).await;
    assert!(
        ref_id.is_some(),
        "line must have inventory_ref_id after close"
    );
}

#[tokio::test]
#[serial]
async fn inbound_close_rejects_qty_mismatch_then_succeeds_after_fix() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    // Create in "receiving" status
    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;

    // Line with mismatched qtys: accepted(7) + rejected(2) = 9 != received(10)
    let line_id = insert_line_full(&pool, tenant_id, ship_id, 10, 7, 2, 10, Some(wh)).await;

    // Attempt close — should fail due to qty mismatch
    let req = make_transition("closed");
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_err(), "close must fail with qty mismatch");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("qty_accepted") || err_msg.contains("qty_rejected"),
        "error must mention qty fields: {err_msg}"
    );

    // Shipment is still in "receiving"
    assert_eq!(get_shipment_status(&pool, ship_id).await, "receiving");

    // Fix: accepted=8, rejected=2, so 8+2=10==received
    sqlx::query("UPDATE shipment_lines SET qty_accepted = 8, qty_rejected = 2 WHERE id = $1")
        .bind(line_id)
        .execute(&pool)
        .await
        .expect("fix line qtys");

    // Retry close — should succeed now
    let result2 = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(
        result2.is_ok(),
        "close must succeed after fix: {:?}",
        result2.err()
    );
    assert_eq!(get_shipment_status(&pool, ship_id).await, "closed");
}

#[tokio::test]
#[serial]
async fn inbound_close_rejects_received_exceeds_expected() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;

    // received=11 > expected=10, accepted+rejected=11==received (guard checks order matters)
    insert_line_full(&pool, tenant_id, ship_id, 10, 11, 0, 11, Some(wh)).await;

    let req = make_transition("closed");
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_err(), "close must fail when received > expected");
}

#[tokio::test]
#[serial]
async fn inbound_outbox_events_emitted_on_transitions() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "draft").await;
    let ship_id_str = ship_id.to_string();
    let line_id = insert_line_full(&pool, tenant_id, ship_id, 5, 0, 0, 0, Some(wh)).await;

    // draft → confirmed
    let req = make_transition("confirmed");
    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("confirmed");
    assert!(
        count_outbox_events(&pool, &ship_id_str, "shipping.shipment.status_changed").await >= 1
    );

    // confirmed → in_transit → arrived → receiving
    for status in &["in_transit", "arrived", "receiving"] {
        let req = make_transition(status);
        ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
            .await
            .unwrap();
    }

    // Record receipt on line and close
    sqlx::query(
        "UPDATE shipment_lines SET qty_received = 5, qty_accepted = 5, qty_rejected = 0 WHERE id = $1",
    )
    .bind(line_id)
    .execute(&pool)
    .await
    .unwrap();

    let req = make_transition("closed");
    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("closed");

    // Verify inbound.closed event specifically
    assert!(
        count_outbox_events(&pool, &ship_id_str, "shipping.inbound.closed").await >= 1,
        "must have inbound.closed outbox event"
    );

    // Total status_changed events: confirmed, in_transit, arrived, receiving = 4
    let status_changes =
        count_outbox_events(&pool, &ship_id_str, "shipping.shipment.status_changed").await;
    assert!(
        status_changes >= 4,
        "expected at least 4 status_changed events, got {status_changes}"
    );
}
