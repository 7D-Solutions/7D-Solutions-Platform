use sqlx::PgPool;

/// Check if an event has already been processed (idempotency check)
pub async fn is_event_processed(
    pool: &PgPool,
    event_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        SELECT event_id
        FROM processed_events
        WHERE event_id = $1
        "#,
        event_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(result.is_some())
}

/// Mark an event as processed to prevent duplicate processing
pub async fn mark_event_processed(
    pool: &PgPool,
    event_id: &str,
    subject: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO processed_events (event_id, subject)
        VALUES ($1, $2)
        ON CONFLICT (event_id) DO NOTHING
        "#,
        event_id,
        subject
    )
    .execute(pool)
    .await?;

    tracing::debug!("Marked event {} as processed", event_id);

    Ok(())
}

/// Process an event with idempotency guarantee
///
/// This function checks if the event has already been processed, and if not,
/// calls the provided handler function and then marks the event as processed.
pub async fn process_event_idempotent<F, Fut>(
    pool: &PgPool,
    event_id: &str,
    subject: &str,
    handler: F,
) -> Result<bool, Box<dyn std::error::Error>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    // Check if already processed
    if is_event_processed(pool, event_id).await? {
        tracing::debug!("Event {} already processed, skipping", event_id);
        return Ok(false);
    }

    // Process the event
    handler().await?;

    // Mark as processed
    mark_event_processed(pool, event_id, subject).await?;

    Ok(true)
}
