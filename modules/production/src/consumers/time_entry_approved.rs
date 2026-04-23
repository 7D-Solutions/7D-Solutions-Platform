use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::cost_tracking::{CostRepo, CostTrackingError, PostCostRequest, PostingCategory};

// ============================================================================
// Local payload mirror (anti-corruption layer)
// mirrors production::events::TimeEntryApprovedPayload
// ============================================================================

#[derive(Debug, serde::Deserialize)]
pub struct TimeEntryApprovedPayload {
    pub time_entry_id: Uuid,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub tenant_id: String,
    pub minutes: i32,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
}

pub type TimeEntryApprovedPayloadTest = TimeEntryApprovedPayload;

// ============================================================================
// Core handler (testable without NATS)
// ============================================================================

pub async fn handle_time_entry_approved(
    pool: &PgPool,
    event_id: Uuid,
    payload: &TimeEntryApprovedPayload,
) -> Result<(), CostTrackingError> {
    // Look up the workcenter cost rate via the operation's workcenter.
    let rate: Option<Option<i64>> = sqlx::query_scalar(
        r#"
        SELECT w.cost_rate_minor
        FROM operations op
        JOIN workcenters w ON w.workcenter_id = op.workcenter_id
        WHERE op.operation_id = $1 AND op.tenant_id = $2
        "#,
    )
    .bind(payload.operation_id)
    .bind(&payload.tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(CostTrackingError::Database)?;

    let cost_rate = match rate {
        Some(Some(r)) => r,
        Some(None) => {
            tracing::warn!(
                time_entry_id = %payload.time_entry_id,
                work_order_id = %payload.work_order_id,
                operation_id = ?payload.operation_id,
                tenant_id = %payload.tenant_id,
                "production.time_entry_approved: workcenter has no cost_rate_minor — skipping labor cost posting"
            );
            return Ok(());
        }
        None => {
            // No operation_id or operation not found — no workcenter to look up.
            tracing::warn!(
                time_entry_id = %payload.time_entry_id,
                work_order_id = %payload.work_order_id,
                operation_id = ?payload.operation_id,
                tenant_id = %payload.tenant_id,
                "production.time_entry_approved: no operation/workcenter found — skipping labor cost posting"
            );
            return Ok(());
        }
    };

    // Labor cost formula: duration_minutes / 60.0 * cost_rate_minor (minor units/hour)
    let amount_cents = ((payload.minutes as f64) / 60.0 * (cost_rate as f64)).round() as i64;

    let req = PostCostRequest {
        work_order_id: payload.work_order_id,
        operation_id: payload.operation_id,
        posting_category: PostingCategory::Labor,
        amount_cents,
        quantity: None,
        source_event_id: Some(event_id),
        posted_by: format!("system:time-entry-consumer:{}", payload.approved_by),
    };

    match CostRepo::post_cost(pool, &req, &payload.tenant_id, &event_id.to_string(), None).await {
        Ok(_) => {
            tracing::info!(
                time_entry_id = %payload.time_entry_id,
                work_order_id = %payload.work_order_id,
                amount_cents,
                "production: labor cost posted"
            );
            Ok(())
        }
        Err(CostTrackingError::DuplicateSourceEvent) => {
            tracing::debug!(
                event_id = %event_id,
                "production.time_entry_approved: duplicate — already posted, skipping"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ============================================================================
// NATS consumer
// ============================================================================

pub fn start_time_entry_approved_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "production.time_entry_approved";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "production: failed to subscribe to time_entry_approved");
                return;
            }
        };

        tracing::info!(
            subject,
            "production: subscribed to time_entry_approved for labor costing"
        );

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_message(&pool, &msg).await {
                tracing::error!(error = %e, "production: failed to process time_entry_approved for costing");
            }
        }

        tracing::warn!("production: time_entry_approved consumer stopped");
    });
}

async fn process_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<TimeEntryApprovedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("failed to parse time_entry_approved envelope: {}", e))?;

    handle_time_entry_approved(pool, envelope.event_id, &envelope.payload).await?;
    Ok(())
}
