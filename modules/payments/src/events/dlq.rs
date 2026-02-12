use sqlx::PgPool;
use uuid::Uuid;

/// Insert a failed event into the Dead Letter Queue
///
/// This function stores events that failed to process after all retries.
/// It ensures no events are silently dropped.
pub async fn insert_failed_event(
    pool: &PgPool,
    event_id: Uuid,
    subject: &str,
    envelope_json: &serde_json::Value,
    error: &str,
    retry_count: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO failed_events (event_id, subject, envelope_json, error, retry_count)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (event_id) DO UPDATE
        SET retry_count = EXCLUDED.retry_count,
            error = EXCLUDED.error,
            failed_at = NOW()
        "#,
    )
    .bind(event_id)
    .bind(subject)
    .bind(envelope_json)
    .bind(error)
    .bind(retry_count)
    .execute(pool)
    .await?;

    tracing::error!(
        event_id = %event_id,
        subject = %subject,
        retry_count = retry_count,
        error = %error,
        "Event moved to DLQ after failure"
    );

    Ok(())
}
