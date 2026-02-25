#![allow(dead_code)]
//! Blue/Green swap orchestration for projection rebuild
//!
//! This module orchestrates the complete rebuild and swap workflow:
//! 1. Create shadow tables
//! 2. Replay events into shadow
//! 3. Verify digest and cursor
//! 4. Atomically swap shadow and live tables

use chrono::Utc;
use projections::{
    compute_digest, create_shadow_cursor_table, create_shadow_table, swap_cursor_tables_atomic, swap_tables_atomic, RebuildResult,
    RebuildSummary,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Configuration for a projection rebuild
#[derive(Debug, Clone)]
pub struct RebuildConfig {
    /// Name of the projection
    pub projection_name: String,

    /// Tenant ID (None for all tenants)
    pub tenant_id: Option<String>,

    /// Base table name (e.g., "customer_balances")
    pub base_table: String,

    /// DDL to create shadow table
    pub create_ddl: String,

    /// Column(s) to order by for digest computation
    pub order_by: String,
}

/// Execute a complete blue/green rebuild
///
/// # Workflow
///
/// 1. Create shadow table
/// 2. Create shadow cursor table
/// 3. Replay events into shadow (caller provides event replay function)
/// 4. Compute digest
/// 5. Atomically swap tables
/// 6. Clean up old tables
///
/// # Arguments
///
/// * `pool` - Database connection pool
/// * `config` - Rebuild configuration
/// * `replay_fn` - Async function that replays events into shadow table
///
/// # Returns
///
/// A `RebuildSummary` with digest and cursor information
pub async fn execute_rebuild<F, Fut>(
    pool: &PgPool,
    config: RebuildConfig,
    replay_fn: F,
) -> RebuildResult<RebuildSummary>
where
    F: FnOnce(PgPool) -> Fut,
    Fut: std::future::Future<Output = RebuildResult<(Uuid, chrono::DateTime<Utc>, i64)>>,
{
    tracing::info!(
        projection = %config.projection_name,
        tenant_id = ?config.tenant_id,
        base_table = %config.base_table,
        "Starting blue/green rebuild"
    );

    // Step 1: Create shadow table
    tracing::info!("Creating shadow table: {}_shadow", config.base_table);
    create_shadow_table(pool, &config.base_table, &config.create_ddl).await?;

    // Step 2: Create shadow cursor table
    tracing::info!("Creating shadow cursor table");
    create_shadow_cursor_table(pool).await?;

    // Step 3: Replay events into shadow
    tracing::info!("Replaying events into shadow table");
    let (last_event_id, last_event_occurred_at, events_processed) = replay_fn(pool.clone()).await?;

    // Step 4: Compute digest of shadow table
    tracing::info!("Computing digest of shadow table");
    let shadow_table = format!("{}_shadow", config.base_table);
    let digest = compute_digest(pool, &shadow_table, &config.order_by).await?;

    tracing::info!(
        digest = %digest,
        events_processed = events_processed,
        "Shadow rebuild complete"
    );

    // Step 5: Atomically swap shadow and live tables
    tracing::info!("Performing atomic blue/green swap");
    swap_tables_atomic(pool, &config.base_table).await?;
    swap_cursor_tables_atomic(pool).await?;

    tracing::info!("Blue/green swap complete - readers now see rebuilt projection");

    // Step 6: Clean up old table (optional - can keep for rollback)
    // For now, we keep the old table for potential rollback
    // drop_shadow_table(pool, &config.base_table).await?;

    Ok(RebuildSummary::new(
        config.projection_name,
        config.tenant_id,
        events_processed,
        last_event_id,
        last_event_occurred_at,
        digest,
    ))
}

/// Verify that a rebuild produced the expected digest
pub async fn verify_rebuild(
    pool: &PgPool,
    base_table: &str,
    order_by: &str,
    expected_digest: &str,
) -> RebuildResult<bool> {
    let actual_digest = compute_digest(pool, base_table, order_by).await?;
    Ok(actual_digest == expected_digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rebuild_config_creation() {
        let config = RebuildConfig {
            projection_name: "customer_balance".to_string(),
            tenant_id: Some("tenant-123".to_string()),
            base_table: "customer_balances".to_string(),
            create_ddl: "CREATE TABLE customer_balances_shadow (id INT)".to_string(),
            order_by: "id".to_string(),
        };

        assert_eq!(config.projection_name, "customer_balance");
        assert_eq!(config.base_table, "customer_balances");
    }
}
