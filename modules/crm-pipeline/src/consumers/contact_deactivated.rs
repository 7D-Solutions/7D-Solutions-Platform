//! Consumer for party.events.contact.deactivated events.
//!
//! Behavior:
//! 1. Set contact_role_attributes.is_active = false
//! 2. Nullify primary_party_contact_id on any opportunity that references this contact

use event_bus::{EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::contact_role_attributes::repo;

#[derive(Debug, serde::Deserialize)]
pub struct ContactDeactivatedPayload {
    pub party_contact_id: Uuid,
    pub tenant_id: String,
}

pub async fn handle_contact_deactivated(pool: &PgPool, payload: &ContactDeactivatedPayload) {
    if let Err(e) = repo::deactivate_contact(pool, &payload.tenant_id, payload.party_contact_id).await {
        tracing::error!(
            tenant_id = %payload.tenant_id,
            party_contact_id = %payload.party_contact_id,
            "CRM: failed to deactivate contact_role_attributes: {}",
            e
        );
    }

    if let Err(e) = repo::nullify_opp_primary_contact(pool, &payload.tenant_id, payload.party_contact_id).await {
        tracing::error!(
            tenant_id = %payload.tenant_id,
            party_contact_id = %payload.party_contact_id,
            "CRM: failed to nullify opportunity primary contact: {}",
            e
        );
    }

    tracing::info!(
        tenant_id = %payload.tenant_id,
        party_contact_id = %payload.party_contact_id,
        "CRM: processed party.contact.deactivated"
    );
}

pub fn start_contact_deactivated_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "party.events.contact.deactivated";
        let mut sub = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("CRM: failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        while let Some(msg) = sub.next().await {
            match serde_json::from_slice::<EventEnvelope<ContactDeactivatedPayload>>(&msg.payload) {
                Ok(envelope) => {
                    handle_contact_deactivated(&pool, &envelope.payload).await;
                }
                Err(e) => {
                    tracing::warn!("CRM: failed to deserialize party.contact.deactivated payload: {}", e);
                }
            }
        }
    });
}
