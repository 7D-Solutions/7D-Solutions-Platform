//! E2E integration tests: Idempotency (bd-2imv).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. Inbound close: repeat close on terminal state → rejected, no duplicate inventory
//! 2. Outbound ship: repeat ship on terminal state → rejected, no duplicate inventory
//! 3. inventory_ref_id is stable after first write (exactly-once)

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
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for integration tests");

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

async fn get_inventory_ref(pool: &sqlx::PgPool, line_id: Uuid) -> Option<Uuid> {
    let row: (Option<Uuid>,) =
        sqlx::query_as("SELECT inventory_ref_id FROM shipment_lines WHERE id = $1")
            .bind(line_id)
            .fetch_one(pool)
            .await
            .expect("get inventory_ref_id");
    row.0
}

async fn count_outbox_events_for(pool: &sqlx::PgPool, aggregate_id: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sr_events_outbox WHERE aggregate_id = $1",
    )
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .expect("count outbox events");
    row.0
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn inbound_close_idempotent_no_duplicate_inventory_or_events() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let ship_id_str = ship_id.to_string();

    let line_id: Uuid = {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO shipment_lines
                (tenant_id, shipment_id, sku, qty_expected, qty_accepted, qty_rejected, qty_received, warehouse_id)
            VALUES ($1, $2, 'IDEM-SKU', 10, 10, 0, 10, $3)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(ship_id)
        .bind(wh)
        .fetch_one(&pool)
        .await
        .expect("insert line");
        row.0
    };

    // First close — succeeds
    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_ok(), "first close must succeed");

    let ref_first = get_inventory_ref(&pool, line_id).await;
    assert!(ref_first.is_some(), "must have inventory ref after close");

    let events_after_first = count_outbox_events_for(&pool, &ship_id_str).await;

    // Second close — must fail (terminal state)
    let result2 = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result2.is_err(), "second close must fail (terminal state)");

    // inventory_ref_id unchanged
    let ref_after = get_inventory_ref(&pool, line_id).await;
    assert_eq!(ref_first, ref_after, "inventory_ref_id must not change on retry");

    // No new outbox events
    let events_after_second = count_outbox_events_for(&pool, &ship_id_str).await;
    assert_eq!(
        events_after_first, events_after_second,
        "no new outbox events after failed retry"
    );
}

#[tokio::test]
#[serial]
async fn outbound_ship_idempotent_no_duplicate_inventory_or_events() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;
    let ship_id_str = ship_id.to_string();

    let line_id: Uuid = {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO shipment_lines
                (tenant_id, shipment_id, sku, qty_expected, qty_shipped, warehouse_id)
            VALUES ($1, $2, 'IDEM-SKU', 8, 8, $3)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(ship_id)
        .bind(wh)
        .fetch_one(&pool)
        .await
        .expect("insert line");
        row.0
    };

    // First ship — succeeds
    let req = TransitionRequest {
        status: "shipped".to_string(),
        arrived_at: None,
        shipped_at: Some(Utc::now()),
        delivered_at: None,
        closed_at: None,
    };
    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_ok(), "first ship must succeed");

    let ref_first = get_inventory_ref(&pool, line_id).await;
    assert!(ref_first.is_some(), "must have inventory ref after ship");

    let events_after_first = count_outbox_events_for(&pool, &ship_id_str).await;

    // Second ship — must fail (already shipped / wrong state)
    let result2 = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result2.is_err(), "second ship must fail");

    // inventory_ref_id unchanged
    let ref_after = get_inventory_ref(&pool, line_id).await;
    assert_eq!(ref_first, ref_after, "inventory_ref_id must not change on retry");

    // No new outbox events
    let events_after_second = count_outbox_events_for(&pool, &ship_id_str).await;
    assert_eq!(
        events_after_first, events_after_second,
        "no new outbox events after failed retry"
    );
}

#[tokio::test]
#[serial]
async fn inventory_ref_exactly_once_per_line() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;

    // Two lines: one with qty, one with zero
    let line_with_qty: Uuid = {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO shipment_lines
                (tenant_id, shipment_id, sku, qty_expected, qty_accepted, qty_rejected, qty_received, warehouse_id)
            VALUES ($1, $2, 'QTY-SKU', 5, 5, 0, 5, $3)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(ship_id)
        .bind(wh)
        .fetch_one(&pool)
        .await
        .expect("insert line with qty");
        row.0
    };

    let line_zero_qty: Uuid = {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO shipment_lines
                (tenant_id, shipment_id, sku, qty_expected, qty_accepted, qty_rejected, qty_received, warehouse_id)
            VALUES ($1, $2, 'ZERO-SKU', 5, 0, 0, 0, $3)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(ship_id)
        .bind(wh)
        .fetch_one(&pool)
        .await
        .expect("insert line zero qty");
        row.0
    };

    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };

    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("close");

    // Line with qty: must have exactly one ref
    let ref_qty = get_inventory_ref(&pool, line_with_qty).await;
    assert!(ref_qty.is_some(), "line with qty must have ref");

    // Line with zero qty: must NOT have ref
    let ref_zero = get_inventory_ref(&pool, line_zero_qty).await;
    assert!(ref_zero.is_none(), "zero-qty line must not have ref");
}
