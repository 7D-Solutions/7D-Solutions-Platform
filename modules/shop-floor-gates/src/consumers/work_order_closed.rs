//! Releases all active holds when a work order is closed.

use event_bus::EventBus;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::holds::{repo as holds_repo, service::SYSTEM_ACTOR};

const SUBJECT: &str = "production.work_order_closed";

#[derive(Debug, Deserialize)]
struct WorkOrderClosedPayload {
    pub tenant_id: String,
    pub work_order_id: Uuid,
}

pub fn start_work_order_closed_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
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
            tracing::error!("SFG: work_order_closed processing error: {}", e);
        }
    }
}

async fn process_message(msg: &event_bus::BusMessage, pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let payload: WorkOrderClosedPayload = serde_json::from_slice(&msg.payload)?;

    // Active holds block WO completion — log a warning if any remain; release them anyway
    let count = holds_repo::count_active_holds_for_work_order(pool, payload.work_order_id, &payload.tenant_id).await?;
    if count > 0 {
        tracing::warn!(
            work_order_id = %payload.work_order_id,
            count,
            "SFG: work order closed with active holds — auto-releasing"
        );
    }

    let released = holds_repo::release_all_active_for_work_order(
        pool,
        payload.work_order_id,
        &payload.tenant_id,
        SYSTEM_ACTOR,
        Some("Auto-released: work order closed"),
    )
    .await?;

    tracing::debug!(work_order_id = %payload.work_order_id, released, "SFG: work_order_closed processed");
    Ok(())
}
