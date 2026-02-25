//! Consumer for AP PO_APPROVED events.
//!
//! When a purchase order is approved, auto-creates an expected inbound shipment
//! with lines from the PO lines. Idempotent via sr_processed_events.
//!
//! ## Anti-corruption layer
//! We define a local mirror of the AP PO_APPROVED payload — we do NOT depend
//! on the AP crate. If the AP event schema changes, only this file needs updating.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repository::{InsertLineParams, InsertShipmentParams, ShipmentRepository};
use crate::domain::shipments::types::InboundStatus;
use crate::outbox;

/// NATS subject for PO approved events (emitted by AP module).
pub const SUBJECT_PO_APPROVED: &str = "ap.po.approved";

// ── Anti-corruption payload ──────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PoApprovedPayload {
    pub tenant_id: String,
    pub po_id: Uuid,
    pub po_number: String,
    pub vendor_id: Uuid,
    pub currency: String,
    pub lines: Vec<PoApprovedLine>,
    pub approved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PoApprovedLine {
    pub line_id: Uuid,
    pub sku: Option<String>,
    pub quantity: i64,
    pub unit_of_measure: Option<String>,
    pub warehouse_id: Option<Uuid>,
}

// ── Public handler (testable without NATS) ───────────────────

/// Process a single PO_APPROVED event.
///
/// Creates an inbound shipment in draft status with one shipment line per PO line.
/// Idempotent: if the event_id was already processed, returns Ok immediately.
pub async fn handle_po_approved(
    pool: &PgPool,
    event_id: Uuid,
    payload: &PoApprovedPayload,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tenant_id: Uuid = payload
        .tenant_id
        .parse()
        .map_err(|e| format!("invalid tenant_id: {e}"))?;

    // Idempotency: skip if already processed
    let mut tx = pool.begin().await?;
    let is_new =
        ShipmentRepository::mark_event_processed_tx(&mut tx, event_id, "ap.po.approved")
            .await?;
    if !is_new {
        tracing::info!(event_id = %event_id, "PO_APPROVED already processed, skipping");
        tx.rollback().await?;
        return Ok(());
    }

    // Create inbound shipment
    let shipment = ShipmentRepository::insert_shipment_tx(
        &mut tx,
        &InsertShipmentParams {
            tenant_id,
            direction: "inbound".to_string(),
            status: InboundStatus::Draft.as_str().to_string(),
            carrier_party_id: None,
            tracking_number: None,
            freight_cost_minor: None,
            currency: Some(payload.currency.clone()),
            expected_arrival_date: None,
            created_by: None,
            source_ref_type: Some("purchase_order".to_string()),
            source_ref_id: Some(payload.po_id),
        },
    )
    .await?;

    tracing::info!(
        shipment_id = %shipment.id,
        po_id = %payload.po_id,
        line_count = payload.lines.len(),
        "Created inbound shipment from PO_APPROVED"
    );

    // Create one shipment line per PO line
    for po_line in &payload.lines {
        ShipmentRepository::insert_line_tx(
            &mut tx,
            &InsertLineParams {
                tenant_id,
                shipment_id: shipment.id,
                sku: po_line.sku.clone(),
                uom: po_line.unit_of_measure.clone(),
                warehouse_id: po_line.warehouse_id,
                qty_expected: po_line.quantity,
                source_ref_type: Some("purchase_order".to_string()),
                source_ref_id: Some(payload.po_id),
                po_id: Some(payload.po_id),
                po_line_id: Some(po_line.line_id),
            },
        )
        .await?;
    }

    // Enqueue ShipmentCreated event to outbox
    let event_payload = serde_json::json!({
        "shipment_id": shipment.id,
        "tenant_id": tenant_id,
        "direction": "inbound",
        "status": "draft",
        "source": "po_approved",
        "po_id": payload.po_id,
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

pub async fn start_po_approved_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("SR: starting PO_APPROVED consumer");

        let mut stream = match bus.subscribe(SUBJECT_PO_APPROVED).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "SR: failed to subscribe to {}", SUBJECT_PO_APPROVED);
                return;
            }
        };

        tracing::info!(subject = SUBJECT_PO_APPROVED, "SR: subscribed");

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_po_approved_message(&pool, &msg).await {
                tracing::error!(error = %e, "SR: failed to process PO_APPROVED event");
            }
        }

        tracing::warn!("SR: PO_APPROVED consumer stopped");
    });
}

async fn process_po_approved_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: EventEnvelope<PoApprovedPayload> =
        serde_json::from_slice(&msg.payload)?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        po_id = %envelope.payload.po_id,
        "SR: processing PO_APPROVED"
    );

    handle_po_approved(pool, envelope.event_id, &envelope.payload).await
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_TENANT: &str = "00000000-0000-0000-0000-000000000099";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://sr_user:sr_pass@localhost:5452/sr_db".to_string()
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
        sqlx::query("DELETE FROM sr_processed_events WHERE event_type = 'ap.po.approved'")
            .execute(pool)
            .await
            .ok();
    }

    fn sample_payload() -> PoApprovedPayload {
        PoApprovedPayload {
            tenant_id: TEST_TENANT.to_string(),
            po_id: Uuid::new_v4(),
            po_number: "PO-TEST-001".to_string(),
            vendor_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            lines: vec![
                PoApprovedLine {
                    line_id: Uuid::new_v4(),
                    sku: Some("WIDGET-A".to_string()),
                    quantity: 10,
                    unit_of_measure: Some("each".to_string()),
                    warehouse_id: Some(Uuid::new_v4()),
                },
                PoApprovedLine {
                    line_id: Uuid::new_v4(),
                    sku: Some("WIDGET-B".to_string()),
                    quantity: 5,
                    unit_of_measure: Some("box".to_string()),
                    warehouse_id: None,
                },
            ],
            approved_at: Utc::now(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_po_approved_creates_inbound_shipment() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let payload = sample_payload();
        let event_id = Uuid::new_v4();
        let tid: Uuid = TEST_TENANT.parse().unwrap();

        handle_po_approved(&pool, event_id, &payload)
            .await
            .expect("handle failed");

        // Verify shipment created
        let shipments = ShipmentRepository::list_shipments(
            &pool, tid, Some("inbound"), None, 10, 0,
        )
        .await
        .expect("list failed");
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].status, "draft");

        // Verify lines created
        let lines = ShipmentRepository::get_lines_for_shipment(
            &pool, shipments[0].id, tid,
        )
        .await
        .expect("get lines failed");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].po_id, Some(payload.po_id));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_po_approved_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let payload = sample_payload();
        let event_id = Uuid::new_v4();
        let tid: Uuid = TEST_TENANT.parse().unwrap();

        handle_po_approved(&pool, event_id, &payload)
            .await
            .expect("first handle failed");
        handle_po_approved(&pool, event_id, &payload)
            .await
            .expect("second handle must not error");

        let shipments = ShipmentRepository::list_shipments(
            &pool, tid, Some("inbound"), None, 10, 0,
        )
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
