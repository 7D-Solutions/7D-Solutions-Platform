//! Event bridge: production → maintenance downtime records
//!
//! Subscribes to:
//! - `production.downtime.started` → create maintenance downtime record
//! - `production.downtime.ended`   → update maintenance downtime record with end_time
//!
//! Idempotent via `maintenance_processed_events` dedup table.
//! Uses production downtime_id as idempotency_key for started events.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::domain::downtime::{CreateDowntimeRequest, DowntimeError, DowntimeRepo};

const PROCESSOR: &str = "production_downtime_bridge";

// ============================================================================
// Production downtime payloads (mirrors production::events contracts)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DowntimeStartedPayload {
    pub downtime_id: Uuid,
    pub tenant_id: String,
    pub workcenter_id: Uuid,
    pub reason: String,
    pub reason_code: Option<String>,
    pub started_at: DateTime<Utc>,
    pub started_by: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DowntimeEndedPayload {
    pub downtime_id: Uuid,
    pub tenant_id: String,
    pub workcenter_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub ended_by: Option<String>,
}

// ============================================================================
// Process downtime.started → create maintenance downtime record
// ============================================================================

pub async fn process_downtime_started(
    pool: &PgPool,
    event_id: Uuid,
    payload: &DowntimeStartedPayload,
) -> Result<Option<Uuid>, String> {
    // Dedup check
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maintenance_processed_events WHERE event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    if already {
        tracing::debug!(event_id = %event_id, "Duplicate downtime.started — skipping");
        return Ok(None);
    }

    // Use production downtime_id as idempotency key to prevent duplicates
    let idem_key = format!("prod-downtime-{}", payload.downtime_id);

    let req = CreateDowntimeRequest {
        tenant_id: payload.tenant_id.clone(),
        asset_id: None,
        start_time: payload.started_at,
        end_time: None,
        reason: payload.reason.clone(),
        impact_classification: "major".to_string(),
        idempotency_key: Some(idem_key),
        notes: payload
            .started_by
            .as_ref()
            .map(|by| format!("Production downtime started by {}", by)),
        workcenter_id: Some(payload.workcenter_id),
        reason_code: payload.reason_code.clone(),
        wo_ref: None,
    };

    let result = DowntimeRepo::create(pool, &req).await;

    let downtime_id = match result {
        Ok(dt) => dt.id,
        Err(DowntimeError::IdempotentDuplicate(dt)) => {
            tracing::info!(
                event_id = %event_id,
                downtime_id = %dt.id,
                "Idempotent duplicate — downtime already exists"
            );
            dt.id
        }
        Err(e) => return Err(format!("Failed to create downtime: {}", e)),
    };

    // Record processed event
    sqlx::query(
        r#"
        INSERT INTO maintenance_processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind("production.downtime.started")
    .bind(PROCESSOR)
    .execute(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    tracing::info!(
        event_id = %event_id,
        downtime_id = %downtime_id,
        workcenter_id = %payload.workcenter_id,
        "Created maintenance downtime from production downtime.started"
    );

    Ok(Some(downtime_id))
}

// ============================================================================
// Process downtime.ended → update maintenance downtime record
// ============================================================================

pub async fn process_downtime_ended(
    pool: &PgPool,
    event_id: Uuid,
    payload: &DowntimeEndedPayload,
) -> Result<(), String> {
    // Dedup check
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maintenance_processed_events WHERE event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    if already {
        tracing::debug!(event_id = %event_id, "Duplicate downtime.ended — skipping");
        return Ok(());
    }

    // Find the maintenance downtime record by idempotency key
    let idem_key = format!("prod-downtime-{}", payload.downtime_id);

    let updated = sqlx::query(
        r#"
        UPDATE downtime_events
        SET end_time = $1
        WHERE tenant_id = $2 AND idempotency_key = $3 AND end_time IS NULL
        "#,
    )
    .bind(payload.ended_at)
    .bind(&payload.tenant_id)
    .bind(&idem_key)
    .execute(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    if updated.rows_affected() == 0 {
        tracing::warn!(
            event_id = %event_id,
            production_downtime_id = %payload.downtime_id,
            "No matching open maintenance downtime found for ended event"
        );
    } else {
        tracing::info!(
            event_id = %event_id,
            production_downtime_id = %payload.downtime_id,
            ended_at = %payload.ended_at,
            "Updated maintenance downtime with end_time from production"
        );
    }

    // Record processed event
    sqlx::query(
        r#"
        INSERT INTO maintenance_processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind("production.downtime.ended")
    .bind(PROCESSOR)
    .execute(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    Ok(())
}

// ============================================================================
// NATS consumers
// ============================================================================

pub async fn start_downtime_bridge(bus: Arc<dyn EventBus>, pool: PgPool) {
    let bus_started = bus.clone();
    let pool_started = pool.clone();
    tokio::spawn(async move {
        tracing::info!("Starting production→maintenance downtime bridge: downtime.started");
        let subject = "production.downtime.started";
        let mut stream = match bus_started.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };
        tracing::info!("Subscribed to {}", subject);

        while let Some(msg) = stream.next().await {
            let span = tracing::info_span!("downtime_bridge_started", subject = %msg.subject);
            let pool = pool_started.clone();
            async move {
                if let Err(e) = process_started_message(&pool, &msg).await {
                    tracing::error!(error = %e, "Downtime bridge (started) processing failed");
                }
            }
            .instrument(span)
            .await;
        }
        tracing::warn!("Downtime started consumer stopped");
    });

    tokio::spawn(async move {
        tracing::info!("Starting production→maintenance downtime bridge: downtime.ended");
        let subject = "production.downtime.ended";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };
        tracing::info!("Subscribed to {}", subject);

        while let Some(msg) = stream.next().await {
            let span = tracing::info_span!("downtime_bridge_ended", subject = %msg.subject);
            let pool = pool.clone();
            async move {
                if let Err(e) = process_ended_message(&pool, &msg).await {
                    tracing::error!(error = %e, "Downtime bridge (ended) processing failed");
                }
            }
            .instrument(span)
            .await;
        }
        tracing::warn!("Downtime ended consumer stopped");
    });
}

async fn process_started_message(pool: &PgPool, msg: &BusMessage) -> Result<(), String> {
    let envelope: EventEnvelope<DowntimeStartedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("Failed to parse downtime.started: {}", e))?;

    process_downtime_started(pool, envelope.event_id, &envelope.payload).await?;
    Ok(())
}

async fn process_ended_message(pool: &PgPool, msg: &BusMessage) -> Result<(), String> {
    let envelope: EventEnvelope<DowntimeEndedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("Failed to parse downtime.ended: {}", e))?;

    process_downtime_ended(pool, envelope.event_id, &envelope.payload).await?;
    Ok(())
}
