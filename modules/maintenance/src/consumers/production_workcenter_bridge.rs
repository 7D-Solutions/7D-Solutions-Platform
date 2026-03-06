//! Event bridge: production → maintenance workcenter projection
//!
//! Subscribes to:
//! - `production.workcenter_created`   → upsert projection
//! - `production.workcenter_updated`   → upsert projection
//! - `production.workcenter_deactivated` → mark inactive
//!
//! Idempotent via `maintenance_processed_events` dedup table.

use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

const PROCESSOR: &str = "production_workcenter_bridge";

// ============================================================================
// Production workcenter payloads (mirrors production::events contracts)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorkcenterCreatedPayload {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorkcenterUpdatedPayload {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorkcenterDeactivatedPayload {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
}

// ============================================================================
// Projection upsert
// ============================================================================

pub async fn upsert_workcenter_projection(
    pool: &PgPool,
    event_id: Uuid,
    workcenter_id: Uuid,
    tenant_id: &str,
    code: &str,
    name: &str,
    is_active: bool,
) -> Result<(), sqlx::Error> {
    // Dedup check
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maintenance_processed_events WHERE event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;

    if already {
        tracing::debug!(event_id = %event_id, "Duplicate workcenter event — skipping");
        return Ok(());
    }

    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO workcenter_projections
            (workcenter_id, tenant_id, code, name, is_active, last_event_id, projected_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (workcenter_id) DO UPDATE
        SET code = EXCLUDED.code,
            name = EXCLUDED.name,
            is_active = EXCLUDED.is_active,
            last_event_id = EXCLUDED.last_event_id,
            projected_at = NOW()
        "#,
    )
    .bind(workcenter_id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(is_active)
    .bind(event_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO maintenance_processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind("production.workcenter")
    .bind(PROCESSOR)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Projection queries
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct WorkcenterProjection {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub is_active: bool,
}

pub async fn list_workcenter_projections(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<WorkcenterProjection>, sqlx::Error> {
    sqlx::query_as::<_, WorkcenterProjection>(
        "SELECT workcenter_id, tenant_id, code, name, is_active FROM workcenter_projections WHERE tenant_id = $1 ORDER BY code",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

// ============================================================================
// NATS consumers
// ============================================================================

pub async fn start_workcenter_bridge(bus: Arc<dyn EventBus>, pool: PgPool) {
    let subjects = [
        "production.workcenter_created",
        "production.workcenter_updated",
        "production.workcenter_deactivated",
    ];

    for subject in subjects {
        let bus = bus.clone();
        let pool = pool.clone();
        let subject = subject.to_string();

        tokio::spawn(async move {
            tracing::info!("Starting production→maintenance workcenter bridge: {}", subject);
            let mut stream = match bus.subscribe(&subject).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to subscribe to {}: {}", subject, e);
                    return;
                }
            };
            tracing::info!("Subscribed to {}", subject);

            while let Some(msg) = stream.next().await {
                let span = tracing::info_span!("workcenter_bridge", subject = %msg.subject);
                let pool = pool.clone();
                async move {
                    if let Err(e) = process_workcenter_message(&pool, &msg).await {
                        tracing::error!(error = %e, "Workcenter bridge processing failed");
                    }
                }
                .instrument(span)
                .await;
            }
            tracing::warn!("Workcenter bridge consumer stopped: {}", subject);
        });
    }
}

async fn process_workcenter_message(pool: &PgPool, msg: &BusMessage) -> Result<(), String> {
    let subject = &msg.subject;

    if subject == "production.workcenter_created" {
        let envelope: EventEnvelope<WorkcenterCreatedPayload> =
            serde_json::from_slice(&msg.payload)
                .map_err(|e| format!("Failed to parse workcenter_created: {}", e))?;

        upsert_workcenter_projection(
            pool,
            envelope.event_id,
            envelope.payload.workcenter_id,
            &envelope.payload.tenant_id,
            &envelope.payload.code,
            &envelope.payload.name,
            true,
        )
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    } else if subject == "production.workcenter_updated" {
        let envelope: EventEnvelope<WorkcenterUpdatedPayload> =
            serde_json::from_slice(&msg.payload)
                .map_err(|e| format!("Failed to parse workcenter_updated: {}", e))?;

        // For updates, we need the existing name since the update payload only has code
        let existing_name: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM workcenter_projections WHERE workcenter_id = $1",
        )
        .bind(envelope.payload.workcenter_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let name = existing_name
            .map(|r| r.0)
            .unwrap_or_else(|| envelope.payload.code.clone());

        upsert_workcenter_projection(
            pool,
            envelope.event_id,
            envelope.payload.workcenter_id,
            &envelope.payload.tenant_id,
            &envelope.payload.code,
            &name,
            true,
        )
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    } else if subject == "production.workcenter_deactivated" {
        let envelope: EventEnvelope<WorkcenterDeactivatedPayload> =
            serde_json::from_slice(&msg.payload)
                .map_err(|e| format!("Failed to parse workcenter_deactivated: {}", e))?;

        let existing_name: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM workcenter_projections WHERE workcenter_id = $1",
        )
        .bind(envelope.payload.workcenter_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        let name = existing_name
            .map(|r| r.0)
            .unwrap_or_else(|| envelope.payload.code.clone());

        upsert_workcenter_projection(
            pool,
            envelope.event_id,
            envelope.payload.workcenter_id,
            &envelope.payload.tenant_id,
            &envelope.payload.code,
            &name,
            false,
        )
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    }

    Ok(())
}
