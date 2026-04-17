//! Cancels all active holds when a work order is cancelled.

use event_bus::EventBus;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::holds::{repo as holds_repo, service::SYSTEM_ACTOR};

const SUBJECT: &str = "production.work_order_cancelled.v1";

#[derive(Debug, Deserialize)]
struct WorkOrderCancelledPayload {
    pub tenant_id: String,
    pub work_order_id: Uuid,
}

pub fn start_work_order_cancelled_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
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
            tracing::error!("SFG: work_order_cancelled processing error: {}", e);
        }
    }
}

async fn process_message(msg: &event_bus::BusMessage, pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let payload: WorkOrderCancelledPayload = serde_json::from_slice(&msg.payload)?;

    let released = holds_repo::release_all_active_for_work_order(
        pool,
        payload.work_order_id,
        &payload.tenant_id,
        SYSTEM_ACTOR,
        Some("Auto-released: work order cancelled"),
    )
    .await?;

    if released > 0 {
        tracing::info!(
            work_order_id = %payload.work_order_id,
            released,
            "SFG: auto-released holds for cancelled work order"
        );
    }

    Ok(())
}
