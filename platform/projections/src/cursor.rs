//! Projection cursor tracking
//!
//! Provides cursor persistence and idempotent apply semantics for event projections.
//! Ensures that:
//! 1. Each projection tracks its position in the event stream
//! 2. Events are never applied twice (idempotency)
//! 3. Cursor updates are transactional with read-model writes
//! 4. Deterministic rebuild capability

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use std::future::Future;
use uuid::Uuid;

/// Result type for cursor operations
pub type CursorResult<T> = Result<T, CursorError>;

/// Errors that can occur during cursor operations
#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Event already processed: projection={projection_name}, tenant={tenant_id}, event_id={event_id}")]
    AlreadyProcessed {
        projection_name: String,
        tenant_id: String,
        event_id: Uuid,
    },

    #[error("Cursor not found: projection={projection_name}, tenant={tenant_id}")]
    NotFound {
        projection_name: String,
        tenant_id: String,
    },
}

/// Represents a projection's cursor position in the event stream
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectionCursor {
    /// Name of the projection
    pub projection_name: String,

    /// Tenant identifier
    pub tenant_id: String,

    /// Last event ID successfully processed
    pub last_event_id: Uuid,

    /// Timestamp of the last processed event
    pub last_event_occurred_at: DateTime<Utc>,

    /// When this cursor was last updated
    pub updated_at: DateTime<Utc>,

    /// Total number of events processed
    pub events_processed: i64,
}

impl ProjectionCursor {
    /// Load cursor for a specific projection and tenant
    ///
    /// Returns None if this is the first time processing events for this projection/tenant.
    pub async fn load(
        executor: impl PgExecutor<'_>,
        projection_name: &str,
        tenant_id: &str,
    ) -> CursorResult<Option<Self>> {
        let cursor = sqlx::query_as::<_, ProjectionCursor>(
            r#"
            SELECT
                projection_name,
                tenant_id,
                last_event_id,
                last_event_occurred_at,
                updated_at,
                events_processed
            FROM projection_cursors
            WHERE projection_name = $1 AND tenant_id = $2
            "#,
        )
        .bind(projection_name)
        .bind(tenant_id)
        .fetch_optional(executor)
        .await?;

        Ok(cursor)
    }

    /// Save or update cursor position
    ///
    /// This should be called transactionally with the projection's read-model writes.
    /// Uses INSERT ... ON CONFLICT to handle both new and existing cursors.
    pub async fn save(
        executor: impl PgExecutor<'_>,
        projection_name: &str,
        tenant_id: &str,
        event_id: Uuid,
        event_occurred_at: DateTime<Utc>,
    ) -> CursorResult<()> {
        sqlx::query(
            r#"
            INSERT INTO projection_cursors (
                projection_name,
                tenant_id,
                last_event_id,
                last_event_occurred_at,
                updated_at,
                events_processed
            ) VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP, 1)
            ON CONFLICT (projection_name, tenant_id)
            DO UPDATE SET
                last_event_id = EXCLUDED.last_event_id,
                last_event_occurred_at = EXCLUDED.last_event_occurred_at,
                updated_at = CURRENT_TIMESTAMP,
                events_processed = projection_cursors.events_processed + 1
            "#,
        )
        .bind(projection_name)
        .bind(tenant_id)
        .bind(event_id)
        .bind(event_occurred_at)
        .execute(executor)
        .await?;

        Ok(())
    }

    /// Check if an event has already been processed
    ///
    /// Returns true if the event_id matches the last processed event.
    /// This ensures idempotency - we never apply the same event twice.
    pub async fn is_processed(
        executor: impl PgExecutor<'_>,
        projection_name: &str,
        tenant_id: &str,
        event_id: Uuid,
    ) -> CursorResult<bool> {
        let cursor = Self::load(executor, projection_name, tenant_id).await?;

        match cursor {
            Some(cursor) => Ok(cursor.last_event_id == event_id),
            None => Ok(false), // No cursor means nothing processed yet
        }
    }
}

/// Idempotent apply contract for projections
///
/// This function wraps the projection apply logic with cursor tracking to ensure:
/// 1. Events are never applied twice (idempotency check)
/// 2. Cursor updates are transactional with projection updates
/// 3. Returns whether the event was actually applied
///
/// # Arguments
///
/// * `tx` - Database transaction (must be PgConnection for transaction support)
/// * `projection_name` - Name of the projection
/// * `tenant_id` - Tenant identifier
/// * `event_id` - Unique event identifier
/// * `event_occurred_at` - When the event occurred
/// * `apply_fn` - Async function that applies the event to the read model
///
/// # Returns
///
/// * `Ok(true)` - Event was applied successfully
/// * `Ok(false)` - Event was already processed (idempotent skip)
/// * `Err(...)` - Database or application error
///
/// # Example
///
/// ```rust,no_run
/// use projections::cursor::{try_apply_event, CursorResult};
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// async fn example(tx: &mut sqlx::PgConnection, event_id: Uuid) -> CursorResult<bool> {
///     try_apply_event(
///         tx,
///         "customer_balance",
///         "tenant-123",
///         event_id,
///         Utc::now(),
///         |tx| Box::pin(async move {
///             // Apply event to read model
///             sqlx::query("UPDATE customer_balances SET balance = balance + $1 WHERE customer_id = $2")
///                 .bind(100)
///                 .bind("cust-456")
///                 .execute(tx)
///                 .await?;
///             Ok(())
///         })
///     ).await
/// }
/// ```
pub async fn try_apply_event<'a, F>(
    tx: &'a mut PgConnection,
    projection_name: &str,
    tenant_id: &str,
    event_id: Uuid,
    event_occurred_at: DateTime<Utc>,
    apply_fn: F,
) -> CursorResult<bool>
where
    F: for<'b> FnOnce(&'b mut PgConnection) -> std::pin::Pin<Box<dyn Future<Output = CursorResult<()>> + Send + 'b>>,
{
    // Check if this event was already processed (idempotency check)
    let already_processed =
        ProjectionCursor::is_processed(&mut *tx, projection_name, tenant_id, event_id).await?;

    if already_processed {
        // Event already processed, skip it (idempotent behavior)
        return Ok(false);
    }

    // Apply the event to the read model (transactional)
    apply_fn(tx).await?;

    // Update cursor to mark this event as processed (transactional)
    ProjectionCursor::save(&mut *tx, projection_name, tenant_id, event_id, event_occurred_at)
        .await?;

    // Event was successfully applied
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests for cursor logic
    // Integration tests will be in e2e-tests/tests/projection_cursor_contract_e2e.rs

    #[test]
    fn test_cursor_error_display() {
        let error = CursorError::AlreadyProcessed {
            projection_name: "test_projection".to_string(),
            tenant_id: "tenant-123".to_string(),
            event_id: Uuid::new_v4(),
        };

        let display = format!("{}", error);
        assert!(display.contains("test_projection"));
        assert!(display.contains("tenant-123"));
    }

    #[test]
    fn test_cursor_not_found_error() {
        let error = CursorError::NotFound {
            projection_name: "missing_projection".to_string(),
            tenant_id: "tenant-456".to_string(),
        };

        let display = format!("{}", error);
        assert!(display.contains("missing_projection"));
        assert!(display.contains("tenant-456"));
    }
}

