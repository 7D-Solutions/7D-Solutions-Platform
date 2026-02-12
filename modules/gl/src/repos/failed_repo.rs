use serde_json::Value as JsonValue;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Insert a failed event into the dead letter queue
pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    subject: &str,
    tenant_id: &str,
    envelope_json: JsonValue,
    error: &str,
    retry_count: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO failed_events (event_id, subject, tenant_id, envelope_json, error, retry_count)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#
    )
    .bind(event_id)
    .bind(subject)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(error)
    .bind(retry_count)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
