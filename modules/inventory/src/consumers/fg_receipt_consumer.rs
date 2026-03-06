//! Consumer for `production.fg_receipt.requested` events.
//!
//! Translates Production's FG receipt requests into Inventory stock receipts.
//! Computes the rolled-up unit cost from component FIFO issue costs for the
//! work order, then calls `process_receipt` with `source_type=production`.
//!
//! Invariant: receipt unit_cost = sum(component issue extended costs) / fg_qty.
//! Zero-cost guard: if no component issues are found, the consumer rejects
//! the receipt to prevent phantom FG entries with no cost basis.

use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::domain::receipt_service::{self, ReceiptRequest, ReceiptResult};

// ============================================================================
// Local payload types (mirrors production::events)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct FgReceiptRequestedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
}

// ============================================================================
// Processing function (testable without NATS)
// ============================================================================

/// Process a single FG receipt request event.
///
/// 1. Queries all component issue costs for the work order from layer_consumptions.
/// 2. Computes rolled-up unit_cost = total_component_cost / fg_quantity.
/// 3. Calls `process_receipt` with source_type=production.
pub async fn process_fg_receipt_request(
    pool: &PgPool,
    event_id: Uuid,
    payload: &FgReceiptRequestedPayload,
    correlation_id: Option<&str>,
    causation_id: Option<&str>,
) -> Result<ReceiptResult, ConsumerError> {
    if payload.quantity <= 0 {
        return Err(ConsumerError::Validation(
            "FG receipt quantity must be > 0".to_string(),
        ));
    }

    // Sum all component issue costs for this work order.
    // Component issues are recorded in the inventory ledger with
    // reference_type = 'production' and reference_id = work_order_id.
    let total_component_cost: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(lc.quantity_consumed * lc.unit_cost_minor), 0)::BIGINT
        FROM layer_consumptions lc
        JOIN inventory_ledger il ON il.id = lc.ledger_entry_id
        WHERE il.tenant_id = $1
          AND il.reference_type = 'production'
          AND il.reference_id = $2
          AND il.entry_type = 'issued'
        "#,
    )
    .bind(&payload.tenant_id)
    .bind(payload.work_order_id.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| ConsumerError::Database(format!("Failed to query component costs: {}", e)))?;

    if total_component_cost <= 0 {
        return Err(ConsumerError::Validation(format!(
            "No component issue costs found for work order {}. Cannot compute FG unit cost.",
            payload.work_order_id
        )));
    }

    // Rolled-up unit cost: integer division (truncate toward zero).
    // For v1 (no variance), this is the actual cost.
    let unit_cost_minor = total_component_cost / payload.quantity;

    if unit_cost_minor <= 0 {
        return Err(ConsumerError::Validation(format!(
            "Computed unit cost is zero or negative (total={}, qty={}). Check component issues.",
            total_component_cost, payload.quantity
        )));
    }

    tracing::info!(
        work_order_id = %payload.work_order_id,
        total_component_cost,
        fg_quantity = payload.quantity,
        unit_cost_minor,
        "Computed rolled-up FG unit cost"
    );

    let corr = correlation_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let idem_key = format!("fg-receipt-{}", event_id);

    let req = ReceiptRequest {
        tenant_id: payload.tenant_id.clone(),
        item_id: payload.item_id,
        warehouse_id: payload.warehouse_id,
        location_id: None,
        quantity: payload.quantity,
        unit_cost_minor,
        currency: payload.currency.clone(),
        source_type: "production".to_string(),
        purchase_order_id: None,
        idempotency_key: idem_key,
        correlation_id: Some(corr),
        causation_id: causation_id.map(|s| s.to_string()),
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    };

    let (result, is_replay) = receipt_service::process_receipt(pool, &req, None)
        .await
        .map_err(|e| ConsumerError::Receipt(format!("Receipt failed: {}", e)))?;

    if is_replay {
        tracing::info!(
            receipt_line_id = %result.receipt_line_id,
            "FG receipt replayed (idempotent)"
        );
    } else {
        tracing::info!(
            receipt_line_id = %result.receipt_line_id,
            unit_cost_minor = result.unit_cost_minor,
            quantity = result.quantity,
            source_type = %result.source_type,
            "FG received into inventory"
        );
    }

    Ok(result)
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the FG receipt consumer.
///
/// Subscribes to `production.fg_receipt.requested` and processes each
/// event by receiving finished goods into Inventory at rolled-up cost.
pub async fn start_fg_receipt_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        run_consumer(bus, pool).await;
    });
}

async fn run_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tracing::info!("Starting FG receipt consumer");

    let subject = "production.fg_receipt.requested";
    let mut stream = match bus.subscribe(subject).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to subscribe to {}: {}", subject, e);
            return;
        }
    };

    tracing::info!("Subscribed to {}", subject);

    while let Some(msg) = stream.next().await {
        let span = tracing::info_span!("process_fg_receipt");
        async {
            if let Err(e) = process_message(&pool, &msg).await {
                tracing::error!(error = %e, "FG receipt processing failed");
            }
        }
        .instrument(span)
        .await;
    }

    tracing::warn!("FG receipt consumer stopped");
}

async fn process_message(pool: &PgPool, msg: &BusMessage) -> Result<(), ConsumerError> {
    let envelope: EventEnvelope<FgReceiptRequestedPayload> =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ConsumerError::Validation(format!("Failed to parse envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        work_order_id = %envelope.payload.work_order_id,
        item_id = %envelope.payload.item_id,
        quantity = envelope.payload.quantity,
        "Processing FG receipt request"
    );

    process_fg_receipt_request(
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

    #[error("Receipt error: {0}")]
    Receipt(String),

    #[error("Database error: {0}")]
    Database(String),
}
