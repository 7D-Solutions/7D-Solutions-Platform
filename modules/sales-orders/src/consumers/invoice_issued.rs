//! Consumer for ar.invoice_opened events.
//! Logs receipt of AR invoice opened notifications.
//! SO order close is handled within the shipment_shipped consumer once
//! all lines are marked invoiced — AR's invoice_opened payload carries no
//! sales-order IDs.

use event_bus::EventBus;
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;

const SUBJECT: &str = "ar.events.ar.invoice_opened";
#[allow(dead_code)]
const QUEUE_GROUP: &str = "sales-orders-invoice-opened";

pub fn start_invoice_issued_consumer(bus: Arc<dyn EventBus>, _pool: PgPool) {
    tokio::spawn(async move {
        consume(bus).await;
    });
}

async fn consume(bus: Arc<dyn EventBus>) {
    let mut stream = match bus.subscribe(SUBJECT).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SO: failed to subscribe to {}: {}", SUBJECT, e);
            return;
        }
    };

    while let Some(msg) = stream.next().await {
        let invoice_id = serde_json::from_slice::<serde_json::Value>(&msg.payload)
            .ok()
            .and_then(|v| v.get("payload").and_then(|p| p.get("invoice_id")).cloned())
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        tracing::debug!(invoice_id = %invoice_id, "SO: ar.invoice_opened received");
    }
}
