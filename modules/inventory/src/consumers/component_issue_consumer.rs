//! Consumer for `production.component_issue.requested` events.
//!
//! Translates Production's component issue requests into Inventory stock issues.
//! Each item in the request is processed via `process_issue` with
//! `source_module=production`, `source_type=production`.
//!
//! The resulting `inventory.item_issued` events carry SourceRef fields linking
//! back to the work order for audit trace.

use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::domain::issue_service::{self, IssueRequest, IssueResult};

// ============================================================================
// Local payload types (mirrors production::events)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentIssueItem {
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentIssueRequestedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub items: Vec<ComponentIssueItem>,
}

// ============================================================================
// Processing function (testable without NATS)
// ============================================================================

/// Process a single component issue request event.
///
/// For each item in the request, calls `process_issue` with production
/// source references. Returns the list of successful issue results.
pub async fn process_component_issue_request(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ComponentIssueRequestedPayload,
    correlation_id: Option<&str>,
    causation_id: Option<&str>,
) -> Result<Vec<IssueResult>, ConsumerError> {
    if payload.items.is_empty() {
        return Err(ConsumerError::Validation(
            "No items in component issue request".to_string(),
        ));
    }

    let corr = correlation_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut results = Vec::with_capacity(payload.items.len());

    for (idx, item) in payload.items.iter().enumerate() {
        let idem_key = format!("ci-{}-{}", event_id, idx);

        let req = IssueRequest {
            tenant_id: payload.tenant_id.clone(),
            item_id: item.item_id,
            warehouse_id: item.warehouse_id,
            location_id: None,
            quantity: item.quantity,
            currency: item.currency.clone(),
            source_module: "production".to_string(),
            source_type: "production".to_string(),
            source_id: payload.work_order_id.to_string(),
            source_line_id: Some(payload.order_number.clone()),
            idempotency_key: idem_key,
            correlation_id: Some(corr.clone()),
            causation_id: causation_id.map(|s| s.to_string()),
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        };

        let (result, _is_replay) = issue_service::process_issue(pool, &req, None)
            .await
            .map_err(|e| ConsumerError::Issue(format!("Issue failed for item {}: {}", idx, e)))?;

        tracing::info!(
            issue_line_id = %result.issue_line_id,
            item_id = %item.item_id,
            quantity = item.quantity,
            total_cost_minor = result.total_cost_minor,
            "Component issued to inventory"
        );

        results.push(result);
    }

    Ok(results)
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the component issue consumer.
///
/// Subscribes to `production.component_issue.requested` and processes each
/// event by issuing stock from Inventory.
pub async fn start_component_issue_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        run_consumer(bus, pool).await;
    });
}

async fn run_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tracing::info!("Starting component issue consumer");

    let subject = "production.component_issue.requested";
    let mut stream = match bus.subscribe(subject).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to subscribe to {}: {}", subject, e);
            return;
        }
    };

    tracing::info!("Subscribed to {}", subject);

    while let Some(msg) = stream.next().await {
        let span = tracing::info_span!("process_component_issue");
        async {
            if let Err(e) = process_message(&pool, &msg).await {
                tracing::error!(error = %e, "Component issue processing failed");
            }
        }
        .instrument(span)
        .await;
    }

    tracing::warn!("Component issue consumer stopped");
}

async fn process_message(pool: &PgPool, msg: &BusMessage) -> Result<(), ConsumerError> {
    let envelope: EventEnvelope<ComponentIssueRequestedPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| ConsumerError::Validation(format!("Failed to parse envelope: {}", e)))?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        work_order_id = %envelope.payload.work_order_id,
        item_count = envelope.payload.items.len(),
        "Processing component issue request"
    );

    process_component_issue_request(
        pool,
        envelope.event_id,
        &envelope.payload,
        envelope.correlation_id.as_deref(),
        envelope.causation_id.as_deref(),
    )
    .await?;

    Ok(())
}

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Issue error: {0}")]
    Issue(String),
}
