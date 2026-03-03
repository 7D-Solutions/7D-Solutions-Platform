//! E2E integration tests: Tenant isolation (bd-2imv).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. Tenant A shipments are invisible to tenant B queries
//! 2. Cross-tenant GET returns not-found / empty
//! 3. Cross-tenant transition fails (not found)
//! 4. List endpoint filters by tenant

use serial_test::serial;
use shipping_receiving_rs::{
    db::repository::ShipmentRepository, domain::shipments::ShipmentService, InventoryIntegration,
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

async fn insert_shipment(pool: &sqlx::PgPool, tenant_id: Uuid, direction: &str) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) VALUES ($1, $2, 'draft') RETURNING id",
    )
    .bind(tenant_id)
    .bind(direction)
    .fetch_one(pool)
    .await
    .expect("insert shipment");
    row.0
}

async fn insert_line(pool: &sqlx::PgPool, tenant_id: Uuid, shipment_id: Uuid) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines (tenant_id, shipment_id, sku, qty_expected)
        VALUES ($1, $2, 'ISO-SKU', 10)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .fetch_one(pool)
    .await
    .expect("insert line");
    row.0
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_shipment_is_tenant_isolated() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    // Tenant A creates a shipment
    let ship_a = insert_shipment(&pool, tenant_a, "inbound").await;

    // Tenant A can see it
    let found = ShipmentService::find_by_id(&pool, ship_a, tenant_a)
        .await
        .expect("query succeeds");
    assert!(found.is_some(), "tenant A must see own shipment");

    // Tenant B cannot see it
    let not_found = ShipmentService::find_by_id(&pool, ship_a, tenant_b)
        .await
        .expect("query succeeds");
    assert!(
        not_found.is_none(),
        "tenant B must NOT see tenant A's shipment"
    );
}

#[tokio::test]
#[serial]
async fn list_shipments_is_tenant_isolated() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    // Tenant A creates two shipments
    insert_shipment(&pool, tenant_a, "inbound").await;
    insert_shipment(&pool, tenant_a, "outbound").await;

    // Tenant B creates one shipment
    let ship_b = insert_shipment(&pool, tenant_b, "outbound").await;

    // Tenant A list: should see exactly 2
    let list_a = ShipmentRepository::list_shipments(&pool, tenant_a, None, None, 100, 0)
        .await
        .expect("list tenant A");
    assert_eq!(list_a.len(), 2, "tenant A must see exactly 2 shipments");

    // Tenant B list: should see exactly 1
    let list_b = ShipmentRepository::list_shipments(&pool, tenant_b, None, None, 100, 0)
        .await
        .expect("list tenant B");
    assert_eq!(list_b.len(), 1, "tenant B must see exactly 1 shipment");
    assert_eq!(list_b[0].id, ship_b);
}

#[tokio::test]
#[serial]
async fn cross_tenant_transition_fails_not_found() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();

    // Tenant A creates inbound draft
    let ship_a = insert_shipment(&pool, tenant_a, "inbound").await;

    // Tenant B tries to transition tenant A's shipment
    let req = shipping_receiving_rs::domain::shipments::TransitionRequest {
        status: "confirmed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: None,
    };

    let result = ShipmentService::transition(&pool, ship_a, tenant_b, &req, &inventory).await;
    assert!(result.is_err(), "cross-tenant transition must fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found") || err_msg.contains("Not found"),
        "error must indicate not found: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn cross_tenant_line_query_returns_empty() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let ship_a = insert_shipment(&pool, tenant_a, "inbound").await;
    insert_line(&pool, tenant_a, ship_a).await;

    // Tenant B queries lines for tenant A's shipment
    let lines = ShipmentRepository::get_lines_for_shipment(&pool, ship_a, tenant_b)
        .await
        .expect("query lines");
    assert!(
        lines.is_empty(),
        "tenant B must NOT see tenant A's shipment lines"
    );
}
