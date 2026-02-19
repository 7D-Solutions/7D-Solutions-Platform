//! Transactional outbox for integrations module.
//!
//! Enqueues events atomically within the caller's transaction into `integrations_outbox`.

use serde::Serialize;
use uuid::Uuid;

/// Enqueue an event into the integrations outbox within a caller-supplied transaction.
///
/// This is the canonical path for Guard→Mutation→Outbox atomicity.
pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    app_id: &str,
    payload: &T,
) -> Result<(), sqlx::Error> {
    let payload_json = serde_json::to_value(payload).map_err(|e| {
        sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize event payload: {}", e),
        )))
    })?;

    sqlx::query(
        r#"
        INSERT INTO integrations_outbox (
            event_id, event_type, aggregate_type, aggregate_id,
            app_id, payload, schema_version
        )
        VALUES ($1, $2, $3, $4, $5, $6, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(app_id)
    .bind(payload_json)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
