//! Consumer for shipping_receiving.shipment_shipped events.
//! Updates shipped_qty on the matching SO line and emits invoice_requested.

use chrono::Utc;
use event_bus::EventBus;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::orders::repo;
use crate::events::{
    build_invoice_requested_envelope, InvoiceRequestedPayload, EVENT_TYPE_INVOICE_REQUESTED,
};
use crate::outbox::enqueue_event_tx;

const SUBJECT: &str = "shipping_receiving.shipment_shipped.v1";
#[allow(dead_code)]
const QUEUE_GROUP: &str = "sales-orders-shipment-shipped";

#[derive(Debug, Deserialize)]
struct ShipmentShippedPayload {
    pub tenant_id: String,
    pub so_line_id: Uuid,
    pub sales_order_id: Uuid,
    pub shipped_qty: f64,
    pub customer_id: Option<Uuid>,
    pub amount_cents: i64,
    pub currency: String,
}

pub fn start_shipment_shipped_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        consume(bus, pool).await;
    });
}

async fn consume(bus: Arc<dyn EventBus>, pool: PgPool) {
    let mut stream = match bus.subscribe(SUBJECT).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SO: failed to subscribe to {}: {}", SUBJECT, e);
            return;
        }
    };

    while let Some(msg) = stream.next().await {
        if let Err(e) = process_message(&msg, &pool).await {
            tracing::error!("SO: shipment_shipped processing error: {}", e);
        }
    }
}

async fn process_message(msg: &event_bus::BusMessage, pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let payload: ShipmentShippedPayload = serde_json::from_slice(&msg.payload)?;

    let mut tx = pool.begin().await?;

    // Update shipped_qty
    repo::update_line_shipped_qty(
        &mut *tx,
        payload.so_line_id,
        &payload.tenant_id,
        payload.shipped_qty,
    )
    .await?;

    // Mark line invoiced
    repo::mark_line_invoiced(&mut *tx, payload.so_line_id, &payload.tenant_id).await?;

    // Emit invoice_requested
    let event_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();
    let envelope = build_invoice_requested_envelope(
        event_id,
        payload.tenant_id.clone(),
        correlation_id,
        None,
        InvoiceRequestedPayload {
            sales_order_id: payload.sales_order_id,
            line_id: payload.so_line_id,
            customer_id: payload.customer_id,
            amount_cents: payload.amount_cents,
            currency: payload.currency,
            tenant_id: payload.tenant_id.clone(),
            requested_at: Utc::now(),
        },
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_INVOICE_REQUESTED,
        "sales_order",
        &payload.sales_order_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}
