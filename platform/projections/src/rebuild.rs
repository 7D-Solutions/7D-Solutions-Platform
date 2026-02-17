//! Deterministic projection rebuild with blue/green swap
//!
//! This module provides infrastructure for rebuilding projections from event sources
//! with atomic blue/green swapping to avoid serving partial state.
//!
//! # Architecture
//!
//! 1. **Shadow Build**: Events are replayed into a shadow table set (e.g., `_shadow` suffix)
//! 2. **Verification**: Cursor position and deterministic digest are computed
//! 3. **Atomic Swap**: Shadow tables are swapped with live tables in one transaction
//!
//! # Determinism
//!
//! The rebuild process is deterministic given:
//! - Same event stream (ordered by event_id or occurred_at)
//! - Same seed data (initial state)
//! - Same projection logic
//!
//! This enables replay certification: two rebuilds should produce identical digests.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

use crate::cursor::{CursorError, CursorResult, ProjectionCursor};

/// Result type for rebuild operations
pub type RebuildResult<T> = Result<T, RebuildError>;

/// Errors that can occur during rebuild
#[derive(Debug, thiserror::Error)]
pub enum RebuildError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Cursor error: {0}")]
    Cursor(#[from] CursorError),

    #[error("Rebuild failed: {0}")]
    Failed(String),

    #[error("Shadow table already exists: {0}")]
    ShadowExists(String),

    #[error("No shadow table to swap: {0}")]
    NoShadow(String),
}

/// Represents the result of a projection rebuild
#[derive(Debug, Clone)]
pub struct RebuildSummary {
    /// Name of the projection
    pub projection_name: String,

    /// Tenant ID (if tenant-scoped)
    pub tenant_id: Option<String>,

    /// Number of events processed during rebuild
    pub events_processed: i64,

    /// Final cursor position
    pub last_event_id: Uuid,

    /// Timestamp of last event
    pub last_event_occurred_at: DateTime<Utc>,

    /// Deterministic digest of the rebuilt projection
    /// (stable across runs with same event stream)
    pub digest: String,

    /// When the rebuild completed
    pub completed_at: DateTime<Utc>,
}

impl RebuildSummary {
    /// Create a new rebuild summary
    pub fn new(
        projection_name: String,
        tenant_id: Option<String>,
        events_processed: i64,
        last_event_id: Uuid,
        last_event_occurred_at: DateTime<Utc>,
        digest: String,
    ) -> Self {
        Self {
            projection_name,
            tenant_id,
            events_processed,
            last_event_id,
            last_event_occurred_at,
            digest,
            completed_at: Utc::now(),
        }
    }
}

/// Create shadow tables for a projection rebuild
///
/// Shadow tables are created with a `_shadow` suffix to isolate the rebuild
/// from the live projection. This allows rebuilding without affecting readers.
///
/// # Arguments
///
/// * `pool` - Database connection pool
/// * `base_table` - Name of the base projection table
/// * `create_ddl` - DDL statement to create the shadow table
///
/// # Example
///
/// ```rust,no_run
/// use projections::rebuild::create_shadow_table;
///
/// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// create_shadow_table(
///     pool,
///     "customer_balances",
///     "CREATE TABLE customer_balances_shadow (
///         customer_id VARCHAR(100) PRIMARY KEY,
///         balance BIGINT NOT NULL
///     )"
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn create_shadow_table(
    pool: &PgPool,
    base_table: &str,
    create_ddl: &str,
) -> RebuildResult<()> {
    // Check if shadow table already exists
    let shadow_table = format!("{}_shadow", base_table);
    let exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = $1
        )
        "#,
    )
    .bind(&shadow_table)
    .fetch_one(pool)
    .await?;

    if exists {
        return Err(RebuildError::ShadowExists(shadow_table));
    }

    // Create shadow table
    sqlx::raw_sql(create_ddl).execute(pool).await?;

    Ok(())
}

/// Drop shadow tables after successful swap
pub async fn drop_shadow_table(pool: &PgPool, base_table: &str) -> RebuildResult<()> {
    let shadow_table = format!("{}_shadow", base_table);
    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", shadow_table))
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically swap shadow and live projection tables (blue/green deployment)
///
/// This function performs an atomic swap of the shadow and live tables in a single
/// transaction. From a reader's perspective, this is one instantaneous step.
///
/// # Arguments
///
/// * `pool` - Database connection pool
/// * `base_table` - Name of the base projection table
///
/// # Transaction Steps
///
/// 1. Rename live table to `_old`
/// 2. Rename shadow table to live name
/// 3. Rename `_old` table to shadow name (for potential rollback)
///
/// This ensures readers always see a complete, consistent projection.
pub async fn swap_tables_atomic(pool: &PgPool, base_table: &str) -> RebuildResult<()> {
    let shadow_table = format!("{}_shadow", base_table);
    let old_table = format!("{}_old", base_table);

    // Check that shadow table exists
    let shadow_exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = $1
        )
        "#,
    )
    .bind(&shadow_table)
    .fetch_one(pool)
    .await?;

    if !shadow_exists {
        return Err(RebuildError::NoShadow(shadow_table));
    }

    // Perform atomic swap in a transaction
    let mut tx = pool.begin().await?;

    // Step 1: Rename live to old (if live exists)
    let live_exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = $1
        )
        "#,
    )
    .bind(base_table)
    .fetch_one(&mut *tx)
    .await?;

    if live_exists {
        // Drop old table if it exists (from previous rebuild)
        sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", old_table))
            .execute(&mut *tx)
            .await?;

        sqlx::query(&format!(
            "ALTER TABLE {} RENAME TO {}",
            base_table, old_table
        ))
        .execute(&mut *tx)
        .await?;
    }

    // Step 2: Rename shadow to live
    sqlx::query(&format!(
        "ALTER TABLE {} RENAME TO {}",
        shadow_table, base_table
    ))
    .execute(&mut *tx)
    .await?;

    // Commit the transaction (atomic swap complete)
    tx.commit().await?;

    Ok(())
}

/// Compute a deterministic digest of a projection
///
/// The digest is computed by hashing the sorted rows of the projection table.
/// This ensures that two rebuilds with the same event stream produce identical digests.
///
/// # Arguments
///
/// * `pool` - Database connection pool
/// * `table_name` - Name of the projection table (or shadow table)
/// * `order_by` - Column(s) to order by for deterministic iteration
///
/// # Returns
///
/// A hex-encoded SHA-256 digest of the projection state
pub async fn compute_digest(
    pool: &PgPool,
    table_name: &str,
    order_by: &str,
) -> RebuildResult<String> {
    // Query all rows in deterministic order
    let query = format!(
        "SELECT * FROM {} ORDER BY {}",
        table_name, order_by
    );

    let mut rows = sqlx::query(&query).fetch(pool);

    let mut hasher = Sha256::new();

    // Stream rows and hash them
    use sqlx::Row;
    while let Some(row) = {
        use futures::StreamExt;
        rows.next().await
    } {
        let row = row?;

        // Hash each column value in order
        for i in 0..row.len() {
            let column_value = row.try_get_raw(i)?;
            hasher.update(column_value.as_bytes().unwrap_or(&[]));
        }
    }

    // Finalize hash
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Load cursor for a projection (from shadow cursor table during rebuild)
pub async fn load_shadow_cursor(
    executor: impl PgExecutor<'_>,
    projection_name: &str,
    tenant_id: &str,
) -> CursorResult<Option<ProjectionCursor>> {
    let cursor = sqlx::query_as::<_, ProjectionCursor>(
        r#"
        SELECT
            projection_name,
            tenant_id,
            last_event_id,
            last_event_occurred_at,
            updated_at,
            events_processed
        FROM projection_cursors_shadow
        WHERE projection_name = $1 AND tenant_id = $2
        "#,
    )
    .bind(projection_name)
    .bind(tenant_id)
    .fetch_optional(executor)
    .await?;

    Ok(cursor)
}

/// Save cursor to shadow cursor table during rebuild
pub async fn save_shadow_cursor(
    executor: impl PgExecutor<'_>,
    projection_name: &str,
    tenant_id: &str,
    event_id: Uuid,
    event_occurred_at: DateTime<Utc>,
) -> CursorResult<()> {
    sqlx::query(
        r#"
        INSERT INTO projection_cursors_shadow (
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
            events_processed = projection_cursors_shadow.events_processed + 1
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

/// Create shadow cursor table for tracking rebuild progress
pub async fn create_shadow_cursor_table(pool: &PgPool) -> RebuildResult<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS projection_cursors_shadow (
            projection_name VARCHAR(100) NOT NULL,
            tenant_id VARCHAR(100) NOT NULL,
            last_event_id UUID NOT NULL,
            last_event_occurred_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            events_processed BIGINT NOT NULL DEFAULT 1,
            PRIMARY KEY (projection_name, tenant_id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Swap cursor tables atomically
pub async fn swap_cursor_tables_atomic(pool: &PgPool) -> RebuildResult<()> {
    let mut tx = pool.begin().await?;

    // Drop old cursor table if exists
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_old CASCADE")
        .execute(&mut *tx)
        .await?;

    // Rename live to old (if exists)
    let live_exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = 'projection_cursors'
        )
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;

    if live_exists {
        sqlx::query("ALTER TABLE projection_cursors RENAME TO projection_cursors_old")
            .execute(&mut *tx)
            .await?;
    }

    // Rename shadow to live
    sqlx::query("ALTER TABLE projection_cursors_shadow RENAME TO projection_cursors")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rebuild_summary_creation() {
        let summary = RebuildSummary::new(
            "test_projection".to_string(),
            Some("tenant-123".to_string()),
            100,
            Uuid::new_v4(),
            Utc::now(),
            "abc123".to_string(),
        );

        assert_eq!(summary.projection_name, "test_projection");
        assert_eq!(summary.tenant_id, Some("tenant-123".to_string()));
        assert_eq!(summary.events_processed, 100);
        assert_eq!(summary.digest, "abc123");
    }

    #[test]
    fn test_rebuild_error_display() {
        let error = RebuildError::ShadowExists("test_table_shadow".to_string());
        let display = format!("{}", error);
        assert!(display.contains("test_table_shadow"));

        let error = RebuildError::NoShadow("test_table_shadow".to_string());
        let display = format!("{}", error);
        assert!(display.contains("test_table_shadow"));
    }
}
