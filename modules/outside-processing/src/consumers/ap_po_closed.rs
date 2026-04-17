//! Consumer: ap.po_closed
//!
//! Log for audit; does not change OP order state.

use event_bus::{BusMessage, EventBus as EventBusTrait};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::repo;

const PROCESSOR: &str = "op_ap_po_closed";
const SUBJECT: &str = "ap.po_closed";

#[derive(Debug, Deserialize)]
struct PoClosedPayload {
    pub tenant_id: String,
    pub event_id: Option<Uuid>,
    pub po_id: Option<Uuid>,
}

pub async fn handle(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload: PoClosedPayload = serde_json::from_slice(&msg.payload)?;

    let Some(event_id) = payload.event_id else {
        return Ok(());
    };

    if repo::is_event_processed(pool, event_id, PROCESSOR).await? {
        return Ok(());
    }

    tracing::info!(
        po_id = ?payload.po_id,
        tenant_id = %payload.tenant_id,
        "OP: ap.po_closed received (audit log only)"
    );

    let mut tx = pool.begin().await?;
    repo::mark_event_processed(&mut tx, event_id, SUBJECT, PROCESSOR).await?;
    tx.commit().await?;

    Ok(())
}

pub fn start_consumer(bus: Arc<dyn EventBusTrait>, pool: PgPool) {
    tokio::spawn(async move {
        let mut sub = match bus.subscribe(SUBJECT).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("OP: failed to subscribe to {}: {}", SUBJECT, e);
                return;
            }
        };
        while let Some(msg) = sub.next().await {
            if let Err(e) = handle(&pool, &msg).await {
                tracing::error!("OP ap_po_closed consumer error: {}", e);
            }
        }
    });
}
