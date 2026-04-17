//! Consumer: inventory.lot_split
//!
//! When a lot linked to an OP order is split, update the OP order's lot_id
//! to reference the child lot.

use event_bus::{BusMessage, EventBus as EventBusTrait};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::repo;

const PROCESSOR: &str = "op_inventory_lot_split";
const SUBJECT: &str = "inventory.lot_split";

#[derive(Debug, Deserialize)]
struct LotSplitPayload {
    pub tenant_id: String,
    pub event_id: Option<Uuid>,
    pub parent_lot_id: Option<Uuid>,
    pub child_lot_id: Option<Uuid>,
}

pub async fn handle(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload: LotSplitPayload = serde_json::from_slice(&msg.payload)?;

    let (Some(parent_lot_id), Some(child_lot_id), Some(event_id)) =
        (payload.parent_lot_id, payload.child_lot_id, payload.event_id) else {
        return Ok(());
    };

    if repo::is_event_processed(pool, event_id, PROCESSOR).await? {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    repo::mark_event_processed(&mut tx, event_id, SUBJECT, PROCESSOR).await?;

    // Update any OP orders referencing the parent lot to the child lot
    sqlx::query(
        "UPDATE op_orders SET lot_id = $1, updated_at = now() WHERE lot_id = $2 AND tenant_id = $3",
    )
    .bind(child_lot_id)
    .bind(parent_lot_id)
    .bind(&payload.tenant_id)
    .execute(&mut *tx)
    .await?;

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
                tracing::error!("OP inventory_lot_split consumer error: {}", e);
            }
        }
    });
}
