//! Idempotency guard for consumer-side event processing.
//!
//! Uses an `event_dedupe` table to track which event IDs have already been
//! processed. The [`with_dedupe`] function wraps an async handler body: if
//! the event has already been seen, the handler is skipped; otherwise the
//! dedupe row is claimed atomically, the handler runs, and on failure the
//! row is removed so the event can be retried.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

/// Outcome of a deduplicated handler invocation.
#[derive(Debug, PartialEq, Eq)]
pub enum DedupeOutcome {
    /// Handler executed — this was the first time we saw this event_id.
    Executed,
    /// Handler skipped — this event_id was already processed.
    Duplicate,
}

/// Execute `handler` exactly once per `event_id`.
///
/// Semantics:
/// 1. Attempt to INSERT into `event_dedupe` with ON CONFLICT DO NOTHING.
/// 2. If 0 rows affected → duplicate, return [`DedupeOutcome::Duplicate`].
/// 3. If 1 row affected → first time; run the handler.
/// 4. If the handler fails → delete the dedupe row (so retry is possible)
///    and propagate the error.
/// 5. If the handler succeeds → dedupe row stays, return [`DedupeOutcome::Executed`].
pub async fn with_dedupe<F, Fut>(
    pool: &PgPool,
    event_id: Uuid,
    subject: &str,
    handler: F,
) -> Result<DedupeOutcome, DedupeError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), DedupeError>>,
{
    let now = Utc::now();

    // Atomic claim: INSERT with ON CONFLICT DO NOTHING.
    // If the row already exists, rows_affected == 0.
    let result = sqlx::query(
        "INSERT INTO event_dedupe (event_id, subject, first_seen_at, last_seen_at)
         VALUES ($1, $2, $3, $3)
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(subject)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| DedupeError::Database(e.to_string()))?;

    if result.rows_affected() == 0 {
        // Update last_seen_at for observability (non-critical, fire-and-forget).
        let _ = sqlx::query("UPDATE event_dedupe SET last_seen_at = $2 WHERE event_id = $1")
            .bind(event_id)
            .bind(now)
            .execute(pool)
            .await;

        tracing::debug!(event_id = %event_id, subject = %subject, "Duplicate event, skipping");
        return Ok(DedupeOutcome::Duplicate);
    }

    // First time seeing this event — run the handler.
    match handler().await {
        Ok(()) => {
            tracing::debug!(event_id = %event_id, subject = %subject, "Event processed (first time)");
            Ok(DedupeOutcome::Executed)
        }
        Err(e) => {
            // Handler failed — remove dedupe row so the event can be retried.
            let _ = sqlx::query("DELETE FROM event_dedupe WHERE event_id = $1")
                .bind(event_id)
                .execute(pool)
                .await;
            tracing::warn!(
                event_id = %event_id,
                subject = %subject,
                error = %e,
                "Handler failed, dedupe row removed for retry"
            );
            Err(e)
        }
    }
}

/// Errors from the deduplication layer.
#[derive(Debug, thiserror::Error)]
pub enum DedupeError {
    /// Database connectivity or query failure.
    #[error("database error: {0}")]
    Database(String),
    /// The handler itself failed.
    #[error("handler error: {0}")]
    Handler(String),
}
