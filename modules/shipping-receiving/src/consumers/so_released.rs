//! Consumer for SO_RELEASED (sales order released) events.
//!
//! When a sales order is released for fulfillment, auto-creates an outbound
//! shipment with lines from the SO lines. Idempotent via sr_processed_events.
//!
//! ## Anti-corruption layer
//! Local mirror of the SO_RELEASED payload — no dependency on AR/sales crate.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repository::{InsertLineParams, InsertShipmentParams, ShipmentRepository};
use crate::domain::shipments::types::OutboundStatus;
use crate::outbox;

/// NATS subject for SO released events (emitted by AR/sales module).
pub const SUBJECT_SO_RELEASED: &str = "sales.so.released";

// ── Anti-corruption payload ──────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SoReleasedPayload {
    pub tenant_id: String,
    pub so_id: Uuid,
    pub so_number: String,
    pub customer_id: Uuid,
    pub currency: String,
    pub ship_to_address: Option<String>,
    pub lines: Vec<SoReleasedLine>,
    pub released_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SoReleasedLine {
    pub line_id: Uuid,
    pub sku: Option<String>,
    pub quantity: i64,
    pub unit_of_measure: Option<String>,
    pub warehouse_id: Option<Uuid>,
}

// ── Public handler (testable without NATS) ───────────────────

/// Process a single SO_RELEASED event.
///
/// Creates an outbound shipment in draft status with one shipment line per SO line.
/// Idempotent: if the event_id was already processed, returns Ok immediately.
pub async fn handle_so_released(
    pool: &PgPool,
    event_id: Uuid,
    payload: &SoReleasedPayload,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tenant_id: Uuid = payload
        .tenant_id
        .parse()
        .map_err(|e| format!("invalid tenant_id: {e}"))?;

    let mut tx = pool.begin().await?;
    let is_new =
        ShipmentRepository::mark_event_processed_tx(&mut tx, event_id, "sales.so.released").await?;
    if !is_new {
        tracing::info!(event_id = %event_id, "SO_RELEASED already processed, skipping");
        tx.rollback().await?;
        return Ok(());
    }

    let shipment = ShipmentRepository::insert_shipment_tx(
        &mut tx,
        &InsertShipmentParams {
            tenant_id,
            direction: "outbound".to_string(),
            status: OutboundStatus::Draft.as_str().to_string(),
            carrier_party_id: None,
            tracking_number: None,
            freight_cost_minor: None,
            currency: Some(payload.currency.clone()),
            expected_arrival_date: None,
            created_by: None,
            source_ref_type: Some("sales_order".to_string()),
            source_ref_id: Some(payload.so_id),
        },
    )
    .await?;

    tracing::info!(
        shipment_id = %shipment.id,
        so_id = %payload.so_id,
        line_count = payload.lines.len(),
        "Created outbound shipment from SO_RELEASED"
    );

    for so_line in &payload.lines {
        ShipmentRepository::insert_line_tx(
            &mut tx,
            &InsertLineParams {
                tenant_id,
                shipment_id: shipment.id,
                sku: so_line.sku.clone(),
                uom: so_line.unit_of_measure.clone(),
                warehouse_id: so_line.warehouse_id,
                qty_expected: so_line.quantity,
                source_ref_type: Some("sales_order".to_string()),
                source_ref_id: Some(payload.so_id),
                po_id: None,
                po_line_id: None,
            },
        )
        .await?;
    }

    let event_payload = serde_json::json!({
        "shipment_id": shipment.id,
        "tenant_id": tenant_id,
        "direction": "outbound",
        "status": "draft",
        "source": "so_released",
        "so_id": payload.so_id,
    });
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        "shipping.shipment.created",
        "shipment",
        &shipment.id.to_string(),
        &tenant_id.to_string(),
        &event_payload,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

// ── NATS consumer (production entry point) ───────────────────

pub async fn start_so_released_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("SR: starting SO_RELEASED consumer");

        let mut stream = match bus.subscribe(SUBJECT_SO_RELEASED).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "SR: failed to subscribe to {}", SUBJECT_SO_RELEASED);
                return;
            }
        };

        tracing::info!(subject = SUBJECT_SO_RELEASED, "SR: subscribed");

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_so_released_message(&pool, &msg).await {
                tracing::error!(error = %e, "SR: failed to process SO_RELEASED event");
            }
        }

        tracing::warn!("SR: SO_RELEASED consumer stopped");
    });
}

async fn process_so_released_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: EventEnvelope<SoReleasedPayload> = serde_json::from_slice(&msg.payload)?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        so_id = %envelope.payload.so_id,
        "SR: processing SO_RELEASED"
    );

    handle_so_released(pool, envelope.event_id, &envelope.payload).await
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_TENANT: &str = "00000000-0000-0000-0000-000000000099";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to SR test DB")
    }

    async fn cleanup(pool: &PgPool) {
        let tid: Uuid = TEST_TENANT.parse().unwrap();
        sqlx::query("DELETE FROM shipment_lines WHERE tenant_id = $1")
            .bind(tid)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM sr_events_outbox WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM shipments WHERE tenant_id = $1")
            .bind(tid)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM sr_processed_events WHERE event_type = 'sales.so.released'")
            .execute(pool)
            .await
            .ok();
    }

    fn sample_payload() -> SoReleasedPayload {
        SoReleasedPayload {
            tenant_id: TEST_TENANT.to_string(),
            so_id: Uuid::new_v4(),
            so_number: "SO-TEST-001".to_string(),
            customer_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            ship_to_address: Some("123 Test St".to_string()),
            lines: vec![SoReleasedLine {
                line_id: Uuid::new_v4(),
                sku: Some("GADGET-X".to_string()),
                quantity: 20,
                unit_of_measure: Some("each".to_string()),
                warehouse_id: Some(Uuid::new_v4()),
            }],
            released_at: Utc::now(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_so_released_creates_outbound_shipment() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let payload = sample_payload();
        let event_id = Uuid::new_v4();
        let tid: Uuid = TEST_TENANT.parse().unwrap();

        handle_so_released(&pool, event_id, &payload)
            .await
            .expect("handle failed");

        let shipments =
            ShipmentRepository::list_shipments(&pool, tid, Some("outbound"), None, 10, 0)
                .await
                .expect("list failed");
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].status, "draft");

        let lines = ShipmentRepository::get_lines_for_shipment(&pool, shipments[0].id, tid)
            .await
            .expect("get lines failed");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].sku.as_deref(), Some("GADGET-X"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_so_released_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let payload = sample_payload();
        let event_id = Uuid::new_v4();
        let tid: Uuid = TEST_TENANT.parse().unwrap();

        handle_so_released(&pool, event_id, &payload)
            .await
            .expect("first handle failed");
        handle_so_released(&pool, event_id, &payload)
            .await
            .expect("second handle must not error");

        let shipments =
            ShipmentRepository::list_shipments(&pool, tid, Some("outbound"), None, 10, 0)
                .await
                .expect("list failed");
        assert_eq!(
            shipments.len(),
            1,
            "idempotent: replay must not create duplicate"
        );

        cleanup(&pool).await;
    }
}
