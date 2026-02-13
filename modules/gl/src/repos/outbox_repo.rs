//! Outbox repository for reliable event publishing
//!
//! Uses the transactional outbox pattern to ensure events are persisted
//! within the same transaction as domain changes.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Insert an event into the outbox for later publishing
pub async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO events_outbox
            (event_id, event_type, aggregate_type, aggregate_id, payload)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
