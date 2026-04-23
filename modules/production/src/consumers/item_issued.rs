use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::cost_tracking::{CostRepo, CostTrackingError, PostCostRequest, PostingCategory};

// ============================================================================
// Local payload mirror (anti-corruption layer)
// mirrors inventory::events::contracts::ItemIssuedPayload
// ============================================================================

#[derive(Debug, serde::Deserialize)]
pub struct ItemIssuedPayload {
    pub issue_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub total_cost_minor: i64,
    pub currency: String,
    pub work_order_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub issued_at: DateTime<Utc>,
}

pub type ItemIssuedPayloadTest = ItemIssuedPayload;

// ============================================================================
// Core handler (testable without NATS)
// ============================================================================

pub async fn handle_item_issued(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ItemIssuedPayload,
) -> Result<(), CostTrackingError> {
    let work_order_id = match payload.work_order_id {
        Some(id) => id,
        None => {
            tracing::debug!(
                issue_line_id = %payload.issue_line_id,
                sku = %payload.sku,
                "inventory.item_issued has no work_order_id; skipping production cost posting"
            );
            return Ok(());
        }
    };

    let req = PostCostRequest {
        work_order_id,
        operation_id: payload.operation_id,
        posting_category: PostingCategory::Material,
        amount_cents: payload.total_cost_minor,
        quantity: Some(payload.quantity as f64),
        source_event_id: Some(event_id),
        posted_by: "system:inventory-item-issued-consumer".to_string(),
    };

    match CostRepo::post_cost(pool, &req, &payload.tenant_id, &event_id.to_string(), None).await {
        Ok(_) => {
            tracing::info!(
                issue_line_id = %payload.issue_line_id,
                work_order_id = %work_order_id,
                amount_cents = payload.total_cost_minor,
                "production: material cost posted from inventory.item_issued"
            );
            Ok(())
        }
        Err(CostTrackingError::DuplicateSourceEvent) => {
            tracing::debug!(
                event_id = %event_id,
                "inventory.item_issued: duplicate — already posted, skipping"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ============================================================================
// NATS consumer
// ============================================================================

pub fn start_item_issued_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "inventory.item_issued";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "production: failed to subscribe to inventory.item_issued");
                return;
            }
        };

        tracing::info!(
            subject,
            "production: subscribed to inventory.item_issued for material costing"
        );

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_message(&pool, &msg).await {
                tracing::error!(error = %e, "production: failed to process inventory.item_issued");
            }
        }

        tracing::warn!("production: inventory.item_issued consumer stopped");
    });
}

async fn process_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<ItemIssuedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("failed to parse inventory.item_issued envelope: {}", e))?;

    handle_item_issued(pool, envelope.event_id, &envelope.payload).await?;
    Ok(())
}
