use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use platform_client_outside_processing::OutsideProcessingClient;
use platform_sdk::PlatformClient;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::cost_tracking::{CostRepo, CostTrackingError, PostCostRequest, PostingCategory};

// ============================================================================
// Local payload mirror (anti-corruption layer)
// mirrors outside_processing::events::produced::OrderClosedPayload
// ============================================================================

#[derive(Debug, serde::Deserialize)]
pub struct OrderClosedPayload {
    pub op_order_id: Uuid,
    pub tenant_id: String,
    pub closed_at: DateTime<Utc>,
    pub final_accepted_qty: i32,
}

// ============================================================================
// Core handler (testable without NATS)
// ============================================================================

pub async fn handle_order_closed(
    pool: &PgPool,
    op_client: &OutsideProcessingClient,
    event_id: Uuid,
    payload: &OrderClosedPayload,
) -> Result<(), CostTrackingError> {
    let claims = PlatformClient::service_claims_from_str(&payload.tenant_id)
        .map_err(|_| CostTrackingError::Database(sqlx::Error::RowNotFound))?;

    let detail = match op_client.get_order(&claims, payload.op_order_id).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(
                op_order_id = %payload.op_order_id,
                error = %e,
                "production: failed to fetch OP order for costing — skipping"
            );
            return Ok(());
        }
    };

    let order = &detail.order;

    let work_order_id = match order.work_order_id {
        Some(id) => id,
        None => {
            tracing::debug!(
                op_order_id = %payload.op_order_id,
                "outside_processing.order_closed: no work_order_id on OP order — skipping production cost posting"
            );
            return Ok(());
        }
    };

    let actual_cost = match order.actual_cost_cents {
        Some(c) => c,
        None => {
            tracing::warn!(
                op_order_id = %payload.op_order_id,
                work_order_id = %work_order_id,
                "outside_processing.order_closed: actual_cost_cents is null — skipping OSP cost posting"
            );
            return Ok(());
        }
    };

    // Prorate cost for partial acceptance.
    let quantity_sent = order.quantity_sent;
    let final_accepted = payload.final_accepted_qty;
    let amount_cents = if quantity_sent > 0 && final_accepted < quantity_sent {
        let ratio = final_accepted as f64 / quantity_sent as f64;
        (actual_cost as f64 * ratio).round() as i64
    } else {
        actual_cost
    };

    let req = PostCostRequest {
        work_order_id,
        operation_id: order.operation_id,
        posting_category: PostingCategory::OutsideProcessing,
        amount_cents,
        quantity: Some(final_accepted as f64),
        source_event_id: Some(event_id),
        posted_by: "system:order-closed-consumer".to_string(),
    };

    match CostRepo::post_cost(pool, &req, &payload.tenant_id, &event_id.to_string(), None).await {
        Ok(_) => {
            tracing::info!(
                op_order_id = %payload.op_order_id,
                work_order_id = %work_order_id,
                amount_cents,
                "production: OSP cost posted from outside_processing.order_closed"
            );
            Ok(())
        }
        Err(CostTrackingError::DuplicateSourceEvent) => {
            tracing::debug!(
                event_id = %event_id,
                "outside_processing.order_closed: duplicate — already posted, skipping"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ============================================================================
// NATS consumer
// ============================================================================

pub fn start_order_closed_consumer(
    bus: Arc<dyn EventBus>,
    pool: PgPool,
    op_client: Arc<OutsideProcessingClient>,
) {
    tokio::spawn(async move {
        let subject = "outside_processing.order_closed";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "production: failed to subscribe to outside_processing.order_closed");
                return;
            }
        };

        tracing::info!(subject, "production: subscribed to outside_processing.order_closed for OSP costing");

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_message(&pool, &op_client, &msg).await {
                tracing::error!(error = %e, "production: failed to process outside_processing.order_closed");
            }
        }

        tracing::warn!("production: outside_processing.order_closed consumer stopped");
    });
}

async fn process_message(
    pool: &PgPool,
    op_client: &OutsideProcessingClient,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<OrderClosedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("failed to parse outside_processing.order_closed envelope: {}", e))?;

    handle_order_closed(pool, op_client, envelope.event_id, &envelope.payload).await?;
    Ok(())
}
