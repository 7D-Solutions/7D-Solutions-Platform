//! Consumer: shipping_receiving.shipment_shipped
//!
//! When Shipping-Receiving confirms a shipment for an OP ship-request,
//! advance the OP order status to shipped_to_vendor (if not already).

use event_bus::{BusMessage, EventBus as EventBusTrait};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::repo;
use crate::domain::state_machine;

const PROCESSOR: &str = "op_shipment_shipped";
const SUBJECT: &str = "shipping_receiving.shipment_shipped";

#[derive(Debug, Deserialize)]
struct ShipmentShippedPayload {
    pub tenant_id: String,
    pub event_id: Option<Uuid>,
    pub op_order_id: Option<Uuid>,
    pub op_ship_event_id: Option<Uuid>,
}

pub async fn handle(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload: ShipmentShippedPayload = serde_json::from_slice(&msg.payload)?;

    let (Some(order_id), Some(event_id)) = (payload.op_order_id, payload.event_id) else {
        return Ok(());
    };

    if repo::is_event_processed(pool, event_id, PROCESSOR).await? {
        return Ok(());
    }

    let order = match repo::get_order(pool, &payload.tenant_id, order_id).await? {
        Some(o) => o,
        None => return Ok(()),
    };

    let new_status = match state_machine::transition_on_ship_event(&order.status) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    let mut tx = pool.begin().await?;
    repo::mark_event_processed(&mut tx, event_id, SUBJECT, PROCESSOR).await?;
    repo::set_order_status(&mut tx, &payload.tenant_id, order_id, new_status.as_str()).await?;
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
                tracing::error!("OP shipment_shipped consumer error: {}", e);
            }
        }
    });
}
