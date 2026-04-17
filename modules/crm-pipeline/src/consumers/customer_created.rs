//! Consumer for ar.customer.created.v1 events.
//!
//! Future enhancement: link AR customer to opportunity/lead.
//! Currently a no-op subscriber that logs receipt.

use event_bus::{EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct ArCustomerCreatedPayload {
    pub customer_id: Uuid,
    pub tenant_id: String,
    pub party_id: Option<Uuid>,
}

pub async fn handle_customer_created(_pool: &PgPool, payload: &ArCustomerCreatedPayload) {
    tracing::debug!(
        tenant_id = %payload.tenant_id,
        customer_id = %payload.customer_id,
        party_id = ?payload.party_id,
        "CRM: received ar.customer.created — future: link to opportunity if party matches"
    );
}

pub fn start_customer_created_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "ar.customer.created.v1";
        let mut sub = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("CRM: failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        while let Some(msg) = sub.next().await {
            match serde_json::from_slice::<EventEnvelope<ArCustomerCreatedPayload>>(&msg.payload) {
                Ok(envelope) => {
                    handle_customer_created(&pool, &envelope.payload).await;
                }
                Err(e) => {
                    tracing::warn!("CRM: failed to deserialize ar.customer.created payload: {}", e);
                }
            }
        }
    });
}
