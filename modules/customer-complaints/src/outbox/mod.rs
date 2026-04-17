use serde::Serialize;
use uuid::Uuid;

/// Enqueue a customer-complaints event into cc_outbox within a caller-supplied transaction.
///
/// Atomicity guarantee: the caller owns the transaction; this function only inserts.
/// Set overdue_emitted_at (or other sentinel columns) in the same transaction.
pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: Uuid,
    tenant_id: &str,
    correlation_id: Option<&str>,
    causation_id: Option<&str>,
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
        INSERT INTO cc_outbox (event_id, event_type, aggregate_type, aggregate_id, tenant_id, payload, correlation_id, causation_id)
        VALUES ($1, $2, 'complaint', $3, $4, $5, $6, $7)
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_id.to_string())
    .bind(tenant_id)
    .bind(payload_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
