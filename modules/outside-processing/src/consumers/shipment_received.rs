//! Consumer: shipping_receiving.shipment_received
//!
//! When Shipping-Receiving receives an inbound shipment that references an OP order,
//! create a return-event stub. Operator completes details via the OP return endpoint.

use event_bus::{BusMessage, EventBus as EventBusTrait};
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::{models::*, repo, state_machine};
use crate::events::{self, *};

const PROCESSOR: &str = "op_shipment_received";
const SUBJECT: &str = "shipping_receiving.shipment_received";

#[derive(Debug, Deserialize)]
struct ShipmentReceivedPayload {
    pub tenant_id: String,
    pub event_id: Option<Uuid>,
    pub op_order_id: Option<Uuid>,
    pub quantity_received: Option<i32>,
    pub received_date: Option<chrono::NaiveDate>,
    pub received_by: Option<String>,
}

pub async fn handle(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload: ShipmentReceivedPayload = serde_json::from_slice(&msg.payload)?;

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

    let new_status = match state_machine::transition_on_return_event(&order.status) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    let req = CreateReturnEventRequest {
        received_date: payload
            .received_date
            .unwrap_or_else(|| chrono::Local::now().date_naive()),
        quantity_received: payload.quantity_received.unwrap_or(0),
        unit_of_measure: None,
        condition: ReturnCondition::Good,
        discrepancy_notes: None,
        lot_number: None,
        serial_numbers: None,
        cert_ref: None,
        vendor_packing_slip: None,
        carrier_name: None,
        tracking_number: None,
        re_identification_required: None,
        received_by: payload.received_by.unwrap_or_else(|| "system".to_string()),
        notes: Some("Auto-created from shipping_receiving.shipment_received".to_string()),
    };

    let mut tx = pool.begin().await?;
    repo::mark_event_processed(&mut tx, event_id, SUBJECT, PROCESSOR).await?;
    let ret_event =
        repo::create_return_event_tx(&mut tx, &payload.tenant_id, order_id, &req).await?;
    repo::set_order_status(&mut tx, &payload.tenant_id, order_id, new_status.as_str()).await?;

    let env_event_id = Uuid::new_v4();
    let env = events::build_returned_envelope(
        env_event_id,
        payload.tenant_id.clone(),
        event_id.to_string(),
        Some(event_id.to_string()),
        ReturnedPayload {
            op_order_id: order_id,
            return_event_id: ret_event.id,
            tenant_id: payload.tenant_id.clone(),
            quantity_received: ret_event.quantity_received,
            condition: ret_event.condition.clone(),
            received_date: ret_event.received_date,
        },
    );
    repo::enqueue_outbox(
        &mut tx,
        &payload.tenant_id,
        env_event_id,
        events::EVENT_RETURNED,
        "op_order",
        &order_id.to_string(),
        &env,
        &event_id.to_string(),
        Some(&event_id.to_string()),
    )
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
                tracing::error!("OP shipment_received consumer error: {}", e);
            }
        }
    });
}
