use chrono::Utc;
use event_bus::EventEnvelope;
use platform_contracts::mutation_classes;
use serde::Serialize;
use sqlx::Postgres;
use sqlx::Transaction;
use uuid::Uuid;

pub async fn enqueue_portal_event<T: Serialize>(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    actor_id: Option<Uuid>,
    event_type: &str,
    payload: T,
) -> Result<(), sqlx::Error> {
    let mut envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "customer-portal".to_string(),
        event_type.to_string(),
        payload,
    );
    envelope.source_version = env!("CARGO_PKG_VERSION").to_string();
    envelope.schema_version = "portal.identity.v1".to_string();
    envelope.replay_safe = true;
    envelope.mutation_class = Some(mutation_classes::LIFECYCLE.to_string());
    envelope.actor_id = actor_id;
    envelope.actor_type = Some(platform_contracts::portal_identity::PORTAL_ACTOR_TYPE.to_string());
    envelope.trace_id = Some(Uuid::new_v4().to_string());
    envelope.correlation_id = Some(Uuid::new_v4().to_string());
    envelope.occurred_at = Utc::now();

    let payload_json = serde_json::to_value(&envelope).map_err(|e| {
        sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("serialize envelope: {e}"),
        )))
    })?;

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(envelope.event_id)
    .bind(event_type)
    .bind("portal_user")
    .bind(tenant_id.to_string())
    .bind(payload_json)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
