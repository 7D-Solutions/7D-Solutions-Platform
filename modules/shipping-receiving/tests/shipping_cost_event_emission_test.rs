//! Integration test: shipping_cost.incurred event emission on label creation.
//!
//! Verifies that `record_label_cost_tx` inserts the correct event into the
//! sr_events_outbox table when a label is recorded. Uses a real PostgreSQL
//! database. No mocks, no stubs.

use serial_test::serial;
use shipping_receiving_rs::http::shipments::create_label::{
    record_label_cost_tx, CreateLabelRequest,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB");
    let _ = sqlx::migrate!("db/migrations").run(&pool).await;
    pool
}

async fn insert_shipment(pool: &sqlx::PgPool, tenant_id: Uuid) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) VALUES ($1, 'outbound', 'packed') RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("insert test shipment");
    id
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: Uuid) {
    sqlx::query(
        "DELETE FROM sr_events_outbox WHERE aggregate_id IN \
         (SELECT id::TEXT FROM shipments WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM shipments WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_create_label_emits_shipping_cost_event() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let shipment_id = insert_shipment(&pool, tenant_id).await;

    let req = CreateLabelRequest {
        tracking_number: "1Z999AA10123456784".to_string(),
        carrier_code: "ups".to_string(),
        carrier_account_ref: Some("ACC-001".to_string()),
        direction: "outbound".to_string(),
        charge_minor: 1250,
        customer_charge_minor: Some(1500),
        currency: "USD".to_string(),
        order_ref: Some("INV-2026-001".to_string()),
    };

    let mut tx = pool.begin().await.expect("begin tx");
    let event_id: Uuid = record_label_cost_tx(&mut tx, shipment_id, tenant_id, &req, "corr-001")
        .await
        .expect("record_label_cost_tx failed");
    tx.commit().await.expect("commit");

    // Assert the event appears in the outbox.
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM sr_events_outbox WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_all(&pool)
    .await
    .expect("outbox query");

    assert_eq!(rows.len(), 1, "exactly one outbox row expected");
    let (event_type, payload) = &rows[0];
    assert_eq!(event_type, "shipping_receiving.shipping_cost.incurred");
    assert_eq!(payload["tracking_number"], "1Z999AA10123456784");
    assert_eq!(payload["carrier_code"], "ups");
    assert_eq!(payload["charge_minor"], 1250);
    assert_eq!(payload["customer_charge_minor"], 1500);
    assert_eq!(payload["currency"], "USD");
    assert_eq!(payload["direction"], "outbound");
    assert_eq!(payload["order_ref"], "INV-2026-001");
    assert_eq!(payload["shipment_id"], shipment_id.to_string());

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_create_label_without_customer_charge() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let shipment_id = insert_shipment(&pool, tenant_id).await;

    let req = CreateLabelRequest {
        tracking_number: "9400111899223397623397".to_string(),
        carrier_code: "usps".to_string(),
        carrier_account_ref: None,
        direction: "outbound".to_string(),
        charge_minor: 800,
        customer_charge_minor: None, // free shipping to customer
        currency: "USD".to_string(),
        order_ref: None,
    };

    let mut tx = pool.begin().await.expect("begin tx");
    let event_id: Uuid = record_label_cost_tx(&mut tx, shipment_id, tenant_id, &req, "corr-002")
        .await
        .expect("record_label_cost_tx failed");
    tx.commit().await.expect("commit");

    let (_, payload): (String, serde_json::Value) = sqlx::query_as(
        "SELECT event_type, payload FROM sr_events_outbox WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox query");

    assert!(payload["customer_charge_minor"].is_null());
    assert!(payload["order_ref"].is_null());

    cleanup(&pool, tenant_id).await;
}
