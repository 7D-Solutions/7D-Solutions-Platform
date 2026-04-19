//! Releases operation-scoped holds when an operation completes.

use event_bus::EventBus;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::holds::service::SYSTEM_ACTOR;

const SUBJECT: &str = "production.operation_completed";

#[derive(Debug, Deserialize)]
struct OperationCompletedPayload {
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
}

pub fn start_operation_completed_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        consume(bus, pool).await;
    });
}

async fn consume(bus: Arc<dyn EventBus>, pool: PgPool) {
    let mut stream = match bus.subscribe(SUBJECT).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SFG: failed to subscribe to {}: {}", SUBJECT, e);
            return;
        }
    };

    while let Some(msg) = stream.next().await {
        if let Err(e) = process_message(&msg, &pool).await {
            tracing::error!("SFG: operation_completed processing error: {}", e);
        }
    }
}

async fn process_message(msg: &event_bus::BusMessage, pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let payload: OperationCompletedPayload = serde_json::from_slice(&msg.payload)?;

    // Release operation-scoped active holds for this specific operation
    let result = sqlx::query(
        r#"UPDATE traveler_holds
           SET status = 'released', released_by = $3, released_at = NOW(),
               release_notes = 'Auto-released: operation completed', updated_at = NOW()
           WHERE work_order_id = $1 AND operation_id = $2 AND tenant_id = $4
             AND scope = 'operation' AND status = 'active'"#,
    )
    .bind(payload.work_order_id)
    .bind(payload.operation_id)
    .bind(SYSTEM_ACTOR)
    .bind(&payload.tenant_id)
    .execute(pool)
    .await?;

    let released = result.rows_affected();
    if released > 0 {
        tracing::info!(
            operation_id = %payload.operation_id,
            released,
            "SFG: auto-released operation-scoped holds"
        );
    }

    Ok(())
}
