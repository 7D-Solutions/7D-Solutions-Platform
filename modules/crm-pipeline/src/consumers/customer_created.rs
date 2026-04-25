//! Consumer for ar.events.ar.customer_created events.
//!
//! When AR creates a customer linked to a party_id, updates open
//! opportunities and leads with the AR customer ID for pipeline enrichment.

use event_bus::{EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct ArCustomerCreatedPayload {
    pub customer_id: i32,
    pub tenant_id: String,
    pub email: String,
    pub name: Option<String>,
    pub party_id: Option<Uuid>,
}

pub async fn handle_customer_created(pool: &PgPool, payload: &ArCustomerCreatedPayload) {
    let Some(party_id) = payload.party_id else {
        return; // No party linkage — nothing to link in CRM
    };

    let opp_result = sqlx::query(
        r#"
        UPDATE opportunities
        SET ar_customer_id = $3, updated_at = NOW()
        WHERE tenant_id = $1 AND party_id = $2 AND ar_customer_id IS NULL
        "#,
    )
    .bind(&payload.tenant_id)
    .bind(party_id)
    .bind(payload.customer_id)
    .execute(pool)
    .await;

    match opp_result {
        Ok(r) if r.rows_affected() > 0 => {
            tracing::info!(
                tenant_id = %payload.tenant_id,
                party_id = %party_id,
                ar_customer_id = %payload.customer_id,
                rows = %r.rows_affected(),
                "CRM: linked AR customer to opportunities"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("CRM: failed to link AR customer to opportunities: {}", e),
    }

    let lead_result = sqlx::query(
        r#"
        UPDATE leads
        SET ar_customer_id = $3, updated_at = NOW()
        WHERE tenant_id = $1 AND party_id = $2 AND ar_customer_id IS NULL
        "#,
    )
    .bind(&payload.tenant_id)
    .bind(party_id)
    .bind(payload.customer_id)
    .execute(pool)
    .await;

    match lead_result {
        Ok(r) if r.rows_affected() > 0 => {
            tracing::info!(
                tenant_id = %payload.tenant_id,
                party_id = %party_id,
                ar_customer_id = %payload.customer_id,
                rows = %r.rows_affected(),
                "CRM: linked AR customer to leads"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("CRM: failed to link AR customer to leads: {}", e),
    }
}

pub fn start_customer_created_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "ar.events.ar.customer_created";
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
                    tracing::warn!(
                        "CRM: failed to deserialize ar.customer_created payload: {}",
                        e
                    );
                }
            }
        }
    });
}
