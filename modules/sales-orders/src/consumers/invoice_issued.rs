//! Consumer for ar.invoice_issued events.
//! Updates the SO status to closed when all lines have been invoiced.

use event_bus::EventBus;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::orders::{repo, SoStatus};

const SUBJECT: &str = "ar.invoice_issued.v1";
#[allow(dead_code)]
const QUEUE_GROUP: &str = "sales-orders-invoice-issued";

#[derive(Debug, Deserialize)]
struct InvoiceIssuedPayload {
    pub tenant_id: String,
    pub so_line_id: Option<Uuid>,
    pub sales_order_id: Option<Uuid>,
}

pub fn start_invoice_issued_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
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
            tracing::error!("SO: invoice_issued processing error: {}", e);
        }
    }
}

async fn process_message(msg: &event_bus::BusMessage, pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let payload: InvoiceIssuedPayload = serde_json::from_slice(&msg.payload)?;

    let (Some(order_id), Some(line_id)) = (payload.sales_order_id, payload.so_line_id) else {
        return Ok(());
    };

    // Check if all lines are invoiced; if so, close the order
    let lines = repo::fetch_lines_for_order(pool, order_id, &payload.tenant_id).await?;
    let all_invoiced = !lines.is_empty() && lines.iter().all(|l| l.invoiced_at.is_some());

    if all_invoiced {
        let order = repo::fetch_order_for_mutation(pool, order_id, &payload.tenant_id).await?;
        if let Some(order) = order {
            let current = SoStatus::from_str(&order.status).unwrap_or(SoStatus::Shipped);
            if current.can_transition_to(SoStatus::Closed) {
                repo::update_order_status(pool, order_id, &payload.tenant_id, SoStatus::Closed.as_str()).await?;
            }
        }
    }

    tracing::debug!(order_id = %order_id, line_id = %line_id, "SO: invoice_issued processed");
    Ok(())
}
