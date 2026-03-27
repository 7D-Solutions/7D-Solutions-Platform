//! E2E integration tests: Outbound shipment workflow (bd-2imv).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. Outbound happy path: draft → confirmed → picking → packed → shipped → delivered → closed
//! 2. Invariant failure: ship with qty_shipped=0 rejects; ship with qty > expected rejects
//! 3. Inventory issue linkage on ship
//! 4. Outbox events emitted for status transitions

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
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db".to_string());

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

async fn insert_outbound_line(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    qty_expected: i64,
    qty_shipped: i64,
    warehouse_id: Option<Uuid>,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_shipped, warehouse_id)
        VALUES ($1, $2, 'OUT-SKU', $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(qty_expected)
    .bind(qty_shipped)
    .bind(warehouse_id)
    .fetch_one(pool)
    .await
    .expect("insert outbound line");
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
        arrived_at: None,
        shipped_at: if status == "shipped" {
            Some(Utc::now())
        } else {
            None
        },
        delivered_at: if status == "delivered" {
            Some(Utc::now())
        } else {
            None
        },
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
async fn outbound_happy_path_draft_to_closed() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "draft").await;

    // Add a line with qty_expected=10, qty_shipped will be set before ship
    let line_id = insert_outbound_line(&pool, tenant_id, ship_id, 10, 0, Some(wh)).await;

    // draft → confirmed → picking → packed
    for status in &["confirmed", "picking", "packed"] {
        let req = make_transition(status);
        let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
        assert!(
            result.is_ok(),
            "transition to {status} failed: {:?}",
            result.err()
        );
    }

    assert_eq!(get_shipment_status(&pool, ship_id).await, "packed");

    // Set qty_shipped on line before shipping
    sqlx::query("UPDATE shipment_lines SET qty_shipped = 10 WHERE id = $1")
        .bind(line_id)
        .execute(&pool)
        .await
        .expect("set qty_shipped");

    // packed → shipped
    let req = make_transition("shipped");
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_ok(), "ship failed: {:?}", result.err());
    assert_eq!(get_shipment_status(&pool, ship_id).await, "shipped");

    // Verify inventory issue linkage
    let ref_id = get_inventory_ref(&pool, line_id).await;
    assert!(
        ref_id.is_some(),
        "line must have inventory_ref_id after ship"
    );

    // shipped → delivered → closed
    for status in &["delivered", "closed"] {
        let req = make_transition(status);
        let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
        assert!(
            result.is_ok(),
            "transition to {status} failed: {:?}",
            result.err()
        );
    }

    assert_eq!(get_shipment_status(&pool, ship_id).await, "closed");
}

#[tokio::test]
#[serial]
async fn outbound_ship_rejects_qty_shipped_zero() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;

    // Line with qty_shipped = 0 — guard should reject
    insert_outbound_line(&pool, tenant_id, ship_id, 10, 0, None).await;

    let req = make_transition("shipped");
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_err(), "ship must fail with qty_shipped=0");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("qty_shipped"),
        "error must mention qty_shipped: {err_msg}"
    );

    // Shipment still in packed
    assert_eq!(get_shipment_status(&pool, ship_id).await, "packed");
}

#[tokio::test]
#[serial]
async fn outbound_ship_rejects_qty_shipped_exceeds_expected() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;

    // Line with qty_shipped=11 > qty_expected=10
    insert_outbound_line(&pool, tenant_id, ship_id, 10, 11, None).await;

    let req = make_transition("shipped");
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(
        result.is_err(),
        "ship must fail when qty_shipped > expected"
    );
}

#[tokio::test]
#[serial]
async fn outbound_outbox_events_emitted_on_transitions() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "draft").await;
    let ship_id_str = ship_id.to_string();
    let line_id = insert_outbound_line(&pool, tenant_id, ship_id, 5, 0, Some(wh)).await;

    // draft → confirmed → picking → packed
    for status in &["confirmed", "picking", "packed"] {
        let req = make_transition(status);
        ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
            .await
            .unwrap();
    }

    // Set qty_shipped and ship
    sqlx::query("UPDATE shipment_lines SET qty_shipped = 5 WHERE id = $1")
        .bind(line_id)
        .execute(&pool)
        .await
        .unwrap();

    let req = make_transition("shipped");
    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("ship");

    // Verify outbound.shipped event
    assert!(
        count_outbox_events(&pool, &ship_id_str, "shipping_receiving.outbound_shipped").await >= 1,
        "must have outbound.shipped outbox event"
    );

    // delivered
    let req = make_transition("delivered");
    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("delivered");

    assert!(
        count_outbox_events(&pool, &ship_id_str, "shipping_receiving.outbound_delivered").await >= 1,
        "must have outbound.delivered outbox event"
    );

    // Total status_changed events: confirmed, picking, packed = 3
    let status_changes =
        count_outbox_events(&pool, &ship_id_str, "shipping_receiving.shipment_status_changed").await;
    assert!(
        status_changes >= 3,
        "expected at least 3 status_changed events, got {status_changes}"
    );
}
