use sqlx::PgPool;
use uuid::Uuid;

/// Check if an event has already been processed (idempotency check)
///
/// Returns true if the event has been processed before, false otherwise.
pub async fn is_event_processed(
    db: &PgPool,
    event_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) as count
        FROM processed_events
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(db)
    .await?;

    Ok(result.0 > 0)
}

/// Mark an event as processed
///
/// This should be called after successfully processing an event to ensure
/// idempotency. If the same event is received again, it will be ignored.
pub async fn mark_event_processed(
    db: &PgPool,
    event_id: Uuid,
    event_type: &str,
    processor: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(processor)
    .execute(db)
    .await?;

    tracing::debug!(
        event_id = %event_id,
        event_type = %event_type,
        processor = %processor,
        "Event marked as processed"
    );

    Ok(())
}

/// Process an event with automatic idempotency checking
///
/// This is a convenience function that:
/// 1. Checks if the event has been processed
/// 2. If not, executes the handler
/// 3. Marks the event as processed
///
/// Returns true if the event was processed, false if it was a duplicate.
pub async fn process_event_idempotent<F, Fut>(
    db: &PgPool,
    event_id: Uuid,
    event_type: &str,
    processor: &str,
    handler: F,
) -> Result<bool, Box<dyn std::error::Error>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    // Check if already processed
    if is_event_processed(db, event_id).await? {
        tracing::info!(
            event_id = %event_id,
            event_type = %event_type,
            "Duplicate event ignored (already processed)"
        );
        return Ok(false);
    }

    // Process the event
    handler().await?;

    // Mark as processed
    mark_event_processed(db, event_id, event_type, processor).await?;

    Ok(true)
}
