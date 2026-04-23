//! Consumer for party.deactivated events.
//!
//! Behavior: log a warning on open opportunities linked to the party.
//! Does not auto-close opportunities per spec.

use event_bus::{EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
pub struct PartyDeactivatedPayload {
    pub party_id: Uuid,
    pub tenant_id: String,
}

pub async fn handle_party_deactivated(pool: &PgPool, payload: &PartyDeactivatedPayload) {
    // Count open opportunities for this party
    let count: Result<i64, _> = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM opportunities o
        JOIN pipeline_stages ps ON ps.tenant_id = o.tenant_id AND ps.stage_code = o.stage_code
        WHERE o.tenant_id = $1 AND o.party_id = $2 AND ps.is_terminal = FALSE
        "#,
    )
    .bind(&payload.tenant_id)
    .bind(payload.party_id)
    .fetch_one(pool)
    .await;

    match count {
        Ok(n) if n > 0 => {
            tracing::warn!(
                tenant_id = %payload.tenant_id,
                party_id = %payload.party_id,
                open_opportunities = n,
                "Party deactivated with open CRM opportunities — manual review required"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!(
            "Failed to query open opportunities for deactivated party: {}",
            e
        ),
    }
}

pub fn start_party_deactivated_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "party.deactivated";
        let mut sub = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("CRM: failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        while let Some(msg) = sub.next().await {
            match serde_json::from_slice::<EventEnvelope<PartyDeactivatedPayload>>(&msg.payload) {
                Ok(envelope) => {
                    handle_party_deactivated(&pool, &envelope.payload).await;
                }
                Err(e) => {
                    tracing::warn!(
                        "CRM: failed to deserialize party.party.deactivated payload: {}",
                        e
                    );
                }
            }
        }
    });
}
