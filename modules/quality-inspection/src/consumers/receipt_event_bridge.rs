//! Event bridge: inventory.item_received → auto-create receiving inspection
//!
//! Subscribes to `inventory.item_received` and creates a receiving inspection
//! for purchase and return receipts. Idempotent — duplicate events are skipped
//! using the `quality_inspection_processed_events` table with event_id as key.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::domain::models::CreateReceivingInspectionRequest;
use crate::domain::service;

const SOURCE_TYPE_PURCHASE: &str = "purchase";
const SOURCE_TYPE_RETURN: &str = "return";
const PROCESSOR_NAME: &str = "receipt_event_bridge";

// ============================================================================
// inventory.item_received payload (mirrors inventory::events::contracts)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ItemReceivedPayload {
    pub receipt_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub currency: String,
    pub source_type: String,
    pub purchase_order_id: Option<Uuid>,
    pub received_at: DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process an `inventory.item_received` event.
///
/// - Only creates inspections for `source_type=purchase` or `source_type=return`.
/// - Idempotent: uses event_id in `quality_inspection_processed_events` to dedup.
/// - Returns `Ok(Some(inspection_id))` if created, `Ok(None)` if skipped.
pub async fn process_item_received(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    payload: &ItemReceivedPayload,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Option<Uuid>, service::QiError> {
    // Only create inspections for purchase or return receipts
    match payload.source_type.as_str() {
        SOURCE_TYPE_PURCHASE | SOURCE_TYPE_RETURN => {}
        other => {
            tracing::debug!(
                event_id = %event_id,
                source_type = %other,
                "Skipping non-purchase/return receipt"
            );
            return Ok(None);
        }
    }

    // Dedup: check if we already processed this event
    let already_processed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM quality_inspection_processed_events WHERE event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;

    if already_processed {
        tracing::info!(
            event_id = %event_id,
            "Duplicate event — already processed, skipping"
        );
        return Ok(None);
    }

    // Create the receiving inspection
    let req = CreateReceivingInspectionRequest {
        plan_id: None,
        receipt_id: Some(payload.receipt_line_id),
        lot_id: None,
        part_id: Some(payload.item_id),
        part_revision: None,
        inspector_id: None,
        result: None, // starts as pending
        notes: Some(format!(
            "Auto-created from inventory receipt (source_type={})",
            payload.source_type
        )),
    };

    let inspection =
        service::create_receiving_inspection(pool, tenant_id, &req, correlation_id, causation_id)
            .await?;

    // Record event as processed (dedup guard)
    sqlx::query(
        r#"
        INSERT INTO quality_inspection_processed_events
            (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind("inventory.item_received")
    .bind(PROCESSOR_NAME)
    .execute(pool)
    .await?;

    tracing::info!(
        event_id = %event_id,
        inspection_id = %inspection.id,
        receipt_line_id = %payload.receipt_line_id,
        source_type = %payload.source_type,
        "Auto-created receiving inspection from inventory receipt"
    );

    Ok(Some(inspection.id))
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the receipt event bridge consumer.
///
/// Subscribes to `inventory.item_received` and auto-creates receiving inspections.
pub async fn start_receipt_event_bridge(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting receipt event bridge consumer");

        let subject = "inventory.item_received";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        tracing::info!("Subscribed to {}", subject);

        while let Some(msg) = stream.next().await {
            let span = tracing::info_span!("receipt_event_bridge", subject = %msg.subject);

            async {
                if let Err(e) = process_message(&pool, &msg).await {
                    tracing::error!(error = %e, "Receipt event bridge processing failed");
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("Receipt event bridge consumer stopped");
    });
}

async fn process_message(pool: &PgPool, msg: &BusMessage) -> Result<(), String> {
    let envelope: EventEnvelope<ItemReceivedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("Failed to parse item_received envelope: {}", e))?;

    let correlation_id = envelope
        .correlation_id
        .as_deref()
        .unwrap_or("none")
        .to_string();

    let causation_event_id = envelope.event_id.to_string();

    process_item_received(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.payload,
        &correlation_id,
        Some(&causation_event_id),
    )
    .await
    .map_err(|e| format!("Bridge processing error: {}", e))?;

    Ok(())
}
