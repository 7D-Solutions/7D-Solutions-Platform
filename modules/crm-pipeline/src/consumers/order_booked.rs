//! Consumer for sales_orders.order_booked events.
//!
//! Behavior: if the SO references an opportunity via soft linkage,
//! update opportunities.sales_order_id. No state change per spec.

use event_bus::{EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct OrderBookedPayload {
    pub sales_order_id: Uuid,
    pub tenant_id: String,
    pub opportunity_id: Option<Uuid>,
}

pub async fn handle_order_booked(pool: &PgPool, payload: &OrderBookedPayload) {
    let Some(opp_id) = payload.opportunity_id else {
        return;
    };

    let result = sqlx::query(
        r#"
        UPDATE opportunities
        SET sales_order_id = $3, updated_at = NOW()
        WHERE tenant_id = $1 AND id = $2 AND sales_order_id IS NULL
        "#,
    )
    .bind(&payload.tenant_id)
    .bind(opp_id)
    .bind(payload.sales_order_id)
    .execute(pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            tracing::info!(
                tenant_id = %payload.tenant_id,
                opportunity_id = %opp_id,
                sales_order_id = %payload.sales_order_id,
                "CRM: linked sales order to opportunity"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("CRM: failed to link sales order to opportunity: {}", e),
    }
}

pub fn start_order_booked_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "sales_orders.order_booked";
        let mut sub = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("CRM: failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        while let Some(msg) = sub.next().await {
            match serde_json::from_slice::<EventEnvelope<OrderBookedPayload>>(&msg.payload) {
                Ok(envelope) => {
                    handle_order_booked(&pool, &envelope.payload).await;
                }
                Err(e) => {
                    tracing::warn!(
                        "CRM: failed to deserialize sales_orders.order.booked payload: {}",
                        e
                    );
                }
            }
        }
    });
}
