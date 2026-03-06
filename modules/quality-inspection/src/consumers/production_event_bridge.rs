//! Event bridge: production → quality auto-create inspections
//!
//! Subscribes to:
//! - `production.operation_completed` → auto-create in-process inspection
//! - `production.fg_receipt.requested` → auto-create final inspection
//!
//! Idempotent — duplicates are skipped using the `quality_inspection_processed_events`
//! table with composite dedup keys: (wo_id, op_instance_id) for in-process,
//! (wo_id, item_id) for final.

use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::domain::models::{CreateFinalInspectionRequest, CreateInProcessInspectionRequest};
use crate::domain::service;

const PROCESSOR_IN_PROCESS: &str = "production_event_bridge_in_process";
const PROCESSOR_FINAL: &str = "production_event_bridge_final";

// ============================================================================
// Production event payloads (mirrors production::events contracts)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OperationCompletedPayload {
    pub operation_id: Uuid,
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub operation_name: String,
    pub sequence_number: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
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
// In-process inspection: production.operation_completed
// ============================================================================

/// Process a `production.operation_completed` event.
///
/// Dedup key: (wo_id, op_instance_id) via processor-scoped event tracking.
/// Returns `Ok(Some(inspection_id))` if created, `Ok(None)` if skipped (duplicate).
pub async fn process_operation_completed(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    payload: &OperationCompletedPayload,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Option<Uuid>, service::QiError> {
    // Dedup: check composite key (wo_id, op_instance_id) via event_id
    let already_processed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM quality_inspection_processed_events WHERE event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;

    if already_processed {
        tracing::info!(
            event_id = %event_id,
            wo_id = %payload.work_order_id,
            op_id = %payload.operation_id,
            "Duplicate operation_completed event — skipping"
        );
        return Ok(None);
    }

    // Also check semantic dedup: same WO + op_instance already has an in-process inspection
    let semantic_dup: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
            SELECT 1 FROM inspections
            WHERE tenant_id = $1 AND wo_id = $2 AND op_instance_id = $3
              AND inspection_type = 'in_process'
        )"#,
    )
    .bind(tenant_id)
    .bind(payload.work_order_id)
    .bind(payload.operation_id)
    .fetch_one(pool)
    .await?;

    if semantic_dup {
        tracing::info!(
            wo_id = %payload.work_order_id,
            op_id = %payload.operation_id,
            "In-process inspection already exists for this WO+op — skipping"
        );
        // Record event as processed so we don't re-check next time
        record_processed_event(pool, event_id, "production.operation_completed", PROCESSOR_IN_PROCESS).await?;
        return Ok(None);
    }

    let req = CreateInProcessInspectionRequest {
        wo_id: payload.work_order_id,
        op_instance_id: payload.operation_id,
        plan_id: None,
        lot_id: None,
        part_id: None,
        part_revision: None,
        inspector_id: None,
        result: None, // starts as pending
        notes: Some(format!(
            "Auto-created from operation completed (op={}, seq={})",
            payload.operation_name, payload.sequence_number
        )),
    };

    let inspection = service::create_in_process_inspection(
        pool,
        tenant_id,
        &req,
        correlation_id,
        causation_id,
    )
    .await?;

    record_processed_event(pool, event_id, "production.operation_completed", PROCESSOR_IN_PROCESS).await?;

    tracing::info!(
        event_id = %event_id,
        inspection_id = %inspection.id,
        wo_id = %payload.work_order_id,
        op_id = %payload.operation_id,
        "Auto-created in-process inspection from operation_completed"
    );

    Ok(Some(inspection.id))
}

// ============================================================================
// Final inspection: production.fg_receipt.requested
// ============================================================================

/// Process a `production.fg_receipt.requested` event.
///
/// Dedup key: (wo_id, item_id) via processor-scoped event tracking.
/// Returns `Ok(Some(inspection_id))` if created, `Ok(None)` if skipped (duplicate).
pub async fn process_fg_receipt_requested(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    payload: &FgReceiptRequestedPayload,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Option<Uuid>, service::QiError> {
    // Dedup: check event_id
    let already_processed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM quality_inspection_processed_events WHERE event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;

    if already_processed {
        tracing::info!(
            event_id = %event_id,
            wo_id = %payload.work_order_id,
            "Duplicate fg_receipt.requested event — skipping"
        );
        return Ok(None);
    }

    let req = CreateFinalInspectionRequest {
        wo_id: payload.work_order_id,
        lot_id: None,
        plan_id: None,
        part_id: Some(payload.item_id),
        part_revision: None,
        inspector_id: None,
        result: None, // starts as pending
        notes: Some(format!(
            "Auto-created from FG receipt request (order={}, qty={})",
            payload.order_number, payload.quantity
        )),
    };

    let inspection = service::create_final_inspection(
        pool,
        tenant_id,
        &req,
        correlation_id,
        causation_id,
    )
    .await?;

    record_processed_event(pool, event_id, "production.fg_receipt.requested", PROCESSOR_FINAL).await?;

    tracing::info!(
        event_id = %event_id,
        inspection_id = %inspection.id,
        wo_id = %payload.work_order_id,
        item_id = %payload.item_id,
        "Auto-created final inspection from fg_receipt.requested"
    );

    Ok(Some(inspection.id))
}

// ============================================================================
// Dedup record helper
// ============================================================================

async fn record_processed_event(
    pool: &PgPool,
    event_id: Uuid,
    event_type: &str,
    processor: &str,
) -> Result<(), service::QiError> {
    sqlx::query(
        r#"
        INSERT INTO quality_inspection_processed_events
            (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(processor)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// NATS consumers (production entry points)
// ============================================================================

/// Start the production event bridge consumers.
///
/// Subscribes to production events and auto-creates inspections.
pub async fn start_production_event_bridge(bus: Arc<dyn EventBus>, pool: PgPool) {
    let bus_op = bus.clone();
    let pool_op = pool.clone();
    tokio::spawn(async move {
        tracing::info!("Starting production→quality bridge: operation_completed consumer");
        let subject = "production.operation_completed";
        let mut stream = match bus_op.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };
        tracing::info!("Subscribed to {}", subject);

        while let Some(msg) = stream.next().await {
            let span = tracing::info_span!("production_event_bridge_op", subject = %msg.subject);
            async {
                if let Err(e) = process_op_completed_message(&pool_op, &msg).await {
                    tracing::error!(error = %e, "Production event bridge (op_completed) processing failed");
                }
            }
            .instrument(span)
            .await;
        }
        tracing::warn!("Operation completed consumer stopped");
    });

    tokio::spawn(async move {
        tracing::info!("Starting production→quality bridge: fg_receipt.requested consumer");
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
            let span = tracing::info_span!("production_event_bridge_fg", subject = %msg.subject);
            async {
                if let Err(e) = process_fg_receipt_message(&pool, &msg).await {
                    tracing::error!(error = %e, "Production event bridge (fg_receipt) processing failed");
                }
            }
            .instrument(span)
            .await;
        }
        tracing::warn!("FG receipt requested consumer stopped");
    });
}

async fn process_op_completed_message(pool: &PgPool, msg: &BusMessage) -> Result<(), String> {
    let envelope: EventEnvelope<OperationCompletedPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| format!("Failed to parse operation_completed envelope: {}", e))?;

    let correlation_id = envelope
        .correlation_id
        .as_deref()
        .unwrap_or("none")
        .to_string();
    let causation_event_id = envelope.event_id.to_string();

    process_operation_completed(
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

async fn process_fg_receipt_message(pool: &PgPool, msg: &BusMessage) -> Result<(), String> {
    let envelope: EventEnvelope<FgReceiptRequestedPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| format!("Failed to parse fg_receipt.requested envelope: {}", e))?;

    let correlation_id = envelope
        .correlation_id
        .as_deref()
        .unwrap_or("none")
        .to_string();
    let causation_event_id = envelope.event_id.to_string();

    process_fg_receipt_requested(
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
