//! Consumer: ap.po_approved
//!
//! If the approved PO was created for an OP order, mark OP as issued (if not already).

use event_bus::{BusMessage, EventBus as EventBusTrait};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::repo;

const PROCESSOR: &str = "op_ap_po_approved";
const SUBJECT: &str = "ap.po_approved";

#[derive(Debug, Deserialize)]
struct PoApprovedPayload {
    pub tenant_id: String,
    pub event_id: Option<Uuid>,
    pub po_id: Option<Uuid>,
}

pub async fn handle(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload: PoApprovedPayload = serde_json::from_slice(&msg.payload)?;

    let (Some(po_id), Some(event_id)) = (payload.po_id, payload.event_id) else {
        return Ok(());
    };

    if repo::is_event_processed(pool, event_id, PROCESSOR).await? {
        return Ok(());
    }

    // Find an OP order referencing this PO
    let order_opt = sqlx::query_as::<_, crate::domain::models::OpOrder>(
        "SELECT * FROM op_orders WHERE purchase_order_id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(po_id)
    .bind(&payload.tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| crate::domain::models::OpError::Database(e))?;

    let Some(order) = order_opt else {
        let mut tx = pool.begin().await?;
        repo::mark_event_processed(&mut tx, event_id, SUBJECT, PROCESSOR).await?;
        tx.commit().await?;
        return Ok(());
    };

    let mut tx = pool.begin().await?;
    repo::mark_event_processed(&mut tx, event_id, SUBJECT, PROCESSOR).await?;
    repo::set_order_status(&mut tx, &payload.tenant_id, order.op_order_id, "issued").await?;
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
                tracing::error!("OP ap_po_approved consumer error: {}", e);
            }
        }
    });
}
