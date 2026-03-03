//! E2E integration tests for inventory integration idempotency (bd-2nft).
//!
//! Tests run against a real PostgreSQL database with deterministic inventory
//! client (no external HTTP calls). Proves:
//! 1. Inbound close creates exactly one receipt per accepted line
//! 2. Outbound ship creates exactly one issue per shipped line
//! 3. Retrying does not duplicate inventory movements
//! 4. Lines with zero qty are skipped
//! 5. inventory_ref_id linkage is persisted on shipment_lines

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

    // Migrations may have been applied by the running service already.
    // Ignore errors from re-running (e.g. "relation already exists").
    let _ = sqlx::migrate!("db/migrations").run(&pool).await;

    pool
}

/// Insert a shipment directly via SQL and return its id.
async fn insert_shipment(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    direction: &str,
    status: &str,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipments (tenant_id, direction, status)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(direction)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert shipment");
    row.0
}

/// Insert a shipment line and return its id.
async fn insert_line(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    qty_expected: i64,
    qty_accepted: i64,
    qty_received: i64,
    qty_shipped: i64,
    warehouse_id: Option<Uuid>,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_accepted,
             qty_received, qty_shipped, warehouse_id)
        VALUES ($1, $2, 'TEST-SKU', $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(qty_expected)
    .bind(qty_accepted)
    .bind(qty_received)
    .bind(qty_shipped)
    .bind(warehouse_id)
    .fetch_one(pool)
    .await
    .expect("insert line");
    row.0
}

/// Insert a shipment line with explicit qty_rejected and return its id.
#[allow(clippy::too_many_arguments)]
async fn insert_line_full(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    qty_expected: i64,
    qty_accepted: i64,
    qty_rejected: i64,
    qty_received: i64,
    qty_shipped: i64,
    warehouse_id: Option<Uuid>,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_accepted,
             qty_rejected, qty_received, qty_shipped, warehouse_id)
        VALUES ($1, $2, 'TEST-SKU', $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(qty_expected)
    .bind(qty_accepted)
    .bind(qty_rejected)
    .bind(qty_received)
    .bind(qty_shipped)
    .bind(warehouse_id)
    .fetch_one(pool)
    .await
    .expect("insert line full");
    row.0
}

/// Read the inventory_ref_id for a line.
async fn get_inventory_ref(pool: &sqlx::PgPool, line_id: Uuid) -> Option<Uuid> {
    let row: (Option<Uuid>,) =
        sqlx::query_as("SELECT inventory_ref_id FROM shipment_lines WHERE id = $1")
            .bind(line_id)
            .fetch_one(pool)
            .await
            .expect("get inventory_ref_id");
    row.0
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn inbound_close_creates_receipts_for_accepted_lines() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    // Create inbound shipment in "receiving" status (ready to close)
    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;

    // Line 1: accepted qty 10 — should get a receipt
    let line1 = insert_line(&pool, tenant_id, ship_id, 10, 10, 10, 0, Some(wh)).await;
    // Line 2: accepted qty 5 — should get a receipt
    let line2 = insert_line(&pool, tenant_id, ship_id, 5, 5, 5, 0, Some(wh)).await;
    // Line 3: accepted qty 0, rejected qty 3 — should be skipped by inventory
    let line3 = insert_line_full(&pool, tenant_id, ship_id, 3, 0, 3, 3, 0, Some(wh)).await;

    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };

    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(
        result.is_ok(),
        "transition should succeed: {:?}",
        result.err()
    );

    // Line 1 and 2 should have inventory_ref_id set
    let ref1 = get_inventory_ref(&pool, line1).await;
    let ref2 = get_inventory_ref(&pool, line2).await;
    let ref3 = get_inventory_ref(&pool, line3).await;

    assert!(
        ref1.is_some(),
        "line1 (qty_accepted=10) must have receipt ref"
    );
    assert!(
        ref2.is_some(),
        "line2 (qty_accepted=5) must have receipt ref"
    );
    assert!(
        ref3.is_none(),
        "line3 (qty_accepted=0) must NOT have receipt ref"
    );

    // Receipts must be different
    assert_ne!(
        ref1, ref2,
        "different lines must have different receipt IDs"
    );
}

#[tokio::test]
#[serial]
async fn outbound_ship_creates_issues_for_shipped_lines() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    // Create outbound shipment in "packed" status (ready to ship)
    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;

    // Line 1: shipped qty 8
    let line1 = insert_line(&pool, tenant_id, ship_id, 10, 0, 0, 8, Some(wh)).await;
    // Line 2: shipped qty 3
    let line2 = insert_line(&pool, tenant_id, ship_id, 5, 0, 0, 3, Some(wh)).await;

    let req = TransitionRequest {
        status: "shipped".to_string(),
        arrived_at: None,
        shipped_at: Some(Utc::now()),
        delivered_at: None,
        closed_at: None,
    };

    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(
        result.is_ok(),
        "transition should succeed: {:?}",
        result.err()
    );

    let ref1 = get_inventory_ref(&pool, line1).await;
    let ref2 = get_inventory_ref(&pool, line2).await;

    assert!(ref1.is_some(), "line1 (qty_shipped=8) must have issue ref");
    assert!(ref2.is_some(), "line2 (qty_shipped=3) must have issue ref");

    // Different lines must have different issue IDs
    assert_ne!(ref1, ref2, "different lines must have different issue IDs");
}

#[tokio::test]
#[serial]
async fn inbound_close_idempotent_no_duplicate_receipts() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    // Create inbound shipment already in "receiving"
    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line1 = insert_line(&pool, tenant_id, ship_id, 10, 10, 10, 0, Some(wh)).await;

    // First close
    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };

    let result = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result.is_ok());

    let ref_first = get_inventory_ref(&pool, line1).await;
    assert!(ref_first.is_some());

    // Attempting to close again should fail (already in terminal "closed" state)
    // The state machine will reject closed→closed transition, proving idempotency
    // at the state-machine level.
    let result2 = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory).await;
    assert!(result2.is_err(), "second close must fail (terminal state)");

    // inventory_ref_id is still the same — not overwritten
    let ref_after = get_inventory_ref(&pool, line1).await;
    assert_eq!(ref_first, ref_after, "inventory_ref_id must not change");
}

#[tokio::test]
#[serial]
async fn inventory_ref_ids_are_deterministic_and_stable() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line1 = insert_line(&pool, tenant_id, ship_id, 10, 10, 10, 0, Some(wh)).await;

    let req = TransitionRequest {
        status: "closed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: Some(Utc::now()),
    };

    let _ = ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("transition");

    let stored_ref = get_inventory_ref(&pool, line1).await.unwrap();

    // Deterministic client: same inputs → same output
    let expected_ref = inventory
        .create_receipt(tenant_id, ship_id, line1, wh, 10, "usd")
        .await
        .unwrap();

    assert_eq!(
        stored_ref, expected_ref,
        "stored ref must match deterministic derivation"
    );
}
