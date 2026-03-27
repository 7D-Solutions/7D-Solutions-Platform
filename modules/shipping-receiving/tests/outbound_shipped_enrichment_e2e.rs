//! E2E tests: Enriched OutboundShippedPayload (bd-31mat).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. tracking_number and carrier_party_id propagate from shipment to event payload
//! 2. source_ref_type and source_ref_id propagate from shipment lines to event lines
//! 3. Backward compatibility: None fields are omitted when not set on the shipment

use chrono::Utc;
use serial_test::serial;
use shipping_receiving_rs::{
    domain::shipments::{ShipmentService, TransitionRequest},
    InventoryIntegration,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

async fn get_outbox_events(
    pool: &sqlx::PgPool,
    aggregate_id: &str,
    event_type: &str,
) -> Vec<serde_json::Value> {
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM sr_events_outbox WHERE aggregate_id = $1 AND event_type = $2 ORDER BY created_at",
    )
    .bind(aggregate_id)
    .bind(event_type)
    .fetch_all(pool)
    .await
    .expect("get outbox events");
    rows.into_iter().map(|(p,)| p).collect()
}

#[tokio::test]
#[serial]
async fn outbound_shipped_enriched_payload_has_tracking_carrier_source_ref() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();
    let carrier_id = Uuid::new_v4();
    let so_id = Uuid::new_v4();

    // Create outbound shipment with tracking_number and carrier_party_id
    let ship_id: Uuid = {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO shipments (tenant_id, direction, status, tracking_number, carrier_party_id)
            VALUES ($1, 'outbound', 'packed', $2, $3)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind("1Z999AA10123456784")
        .bind(carrier_id)
        .fetch_one(&pool)
        .await
        .expect("insert shipment with tracking");
        row.0
    };

    // Insert line with source_ref
    sqlx::query(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_shipped, warehouse_id,
             source_ref_type, source_ref_id)
        VALUES ($1, $2, 'ENRICH-SKU', 5, 5, $3, 'sales_order', $4)
        "#,
    )
    .bind(tenant_id)
    .bind(ship_id)
    .bind(wh)
    .bind(so_id)
    .execute(&pool)
    .await
    .expect("insert line with source_ref");

    let req = TransitionRequest {
        status: "shipped".to_string(),
        arrived_at: None,
        shipped_at: Some(Utc::now()),
        delivered_at: None,
        closed_at: None,
    };

    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("ship");

    let events = get_outbox_events(
        &pool,
        &ship_id.to_string(),
        "shipping_receiving.outbound_shipped",
    )
    .await;
    assert_eq!(events.len(), 1, "exactly one outbound_shipped event");

    let payload = &events[0];

    // Shipment-level enrichment
    assert_eq!(
        payload["tracking_number"].as_str(),
        Some("1Z999AA10123456784"),
        "tracking_number must be populated from shipment"
    );
    assert_eq!(
        payload["carrier_party_id"].as_str(),
        Some(carrier_id.to_string()).as_deref(),
        "carrier_party_id must be populated from shipment"
    );

    // Line-level enrichment
    let lines = payload["lines"].as_array().expect("lines must exist");
    assert!(!lines.is_empty(), "must have at least one line");
    let line = &lines[0];
    assert_eq!(
        line["source_ref_type"].as_str(),
        Some("sales_order"),
        "line source_ref_type must be populated"
    );
    assert_eq!(
        line["source_ref_id"].as_str(),
        Some(so_id.to_string()).as_deref(),
        "line source_ref_id must be populated"
    );
}

#[tokio::test]
#[serial]
async fn outbound_shipped_backward_compat_none_fields() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let wh = Uuid::new_v4();

    // Create outbound shipment WITHOUT tracking_number or carrier
    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;

    // Insert line WITHOUT source_ref
    sqlx::query(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_shipped, warehouse_id)
        VALUES ($1, $2, 'COMPAT-SKU', 3, 3, $3)
        "#,
    )
    .bind(tenant_id)
    .bind(ship_id)
    .bind(wh)
    .execute(&pool)
    .await
    .expect("insert line without source_ref");

    let req = TransitionRequest {
        status: "shipped".to_string(),
        arrived_at: None,
        shipped_at: Some(Utc::now()),
        delivered_at: None,
        closed_at: None,
    };

    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("ship");

    let events = get_outbox_events(
        &pool,
        &ship_id.to_string(),
        "shipping_receiving.outbound_shipped",
    )
    .await;
    assert_eq!(events.len(), 1, "exactly one outbound_shipped event");

    let payload = &events[0];

    // Payload must deserialize successfully — new Optional fields absent or null
    assert_eq!(payload["shipment_id"], ship_id.to_string());
    assert!(payload["shipped_at"].is_string(), "must have shipped_at");
    assert!(
        payload.get("tracking_number").is_none() || payload["tracking_number"].is_null(),
        "tracking_number must be absent or null when not set"
    );
    assert!(
        payload.get("carrier_party_id").is_none() || payload["carrier_party_id"].is_null(),
        "carrier_party_id must be absent or null when not set"
    );

    // Lines must exist and have no source_ref
    let lines = payload["lines"].as_array().expect("lines must exist");
    assert!(!lines.is_empty());
    let line = &lines[0];
    assert!(
        line.get("source_ref_type").is_none() || line["source_ref_type"].is_null(),
        "source_ref_type must be absent or null when not set"
    );
    assert!(
        line.get("source_ref_id").is_none() || line["source_ref_id"].is_null(),
        "source_ref_id must be absent or null when not set"
    );
}
