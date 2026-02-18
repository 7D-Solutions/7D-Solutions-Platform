//! Query functions for valuation snapshot read endpoints.
//!
//! All queries enforce tenant scoping via `tenant_id` in WHERE clauses.
//! These are read-only; no state is mutated.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::{ValuationLine, ValuationSnapshot};

// ============================================================================
// List snapshots
// ============================================================================

/// List valuation snapshots for a tenant, newest first.
///
/// Optional `warehouse_id` narrows results to a specific warehouse.
/// Returns up to `limit` rows starting at `offset`.
pub async fn list_snapshots(
    pool: &PgPool,
    tenant_id: &str,
    warehouse_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ValuationSnapshot>, sqlx::Error> {
    if let Some(wh) = warehouse_id {
        sqlx::query_as::<_, ValuationSnapshot>(
            r#"
            SELECT id, tenant_id, warehouse_id, location_id, as_of,
                   total_value_minor, currency, created_at
            FROM inventory_valuation_snapshots
            WHERE tenant_id = $1 AND warehouse_id = $2
            ORDER BY as_of DESC, created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(tenant_id)
        .bind(wh)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ValuationSnapshot>(
            r#"
            SELECT id, tenant_id, warehouse_id, location_id, as_of,
                   total_value_minor, currency, created_at
            FROM inventory_valuation_snapshots
            WHERE tenant_id = $1
            ORDER BY as_of DESC, created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }
}

// ============================================================================
// Get snapshot detail
// ============================================================================

/// Fetch a snapshot header by id, tenant-scoped.
///
/// Returns `None` when the snapshot does not exist or belongs to a different tenant.
pub async fn get_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    snapshot_id: Uuid,
) -> Result<Option<ValuationSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, ValuationSnapshot>(
        r#"
        SELECT id, tenant_id, warehouse_id, location_id, as_of,
               total_value_minor, currency, created_at
        FROM inventory_valuation_snapshots
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(snapshot_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Fetch all lines for a snapshot ordered by item_id (deterministic).
///
/// The caller is responsible for ensuring the snapshot belongs to the correct
/// tenant before calling this function (via `get_snapshot`).
pub async fn get_snapshot_lines(
    pool: &PgPool,
    snapshot_id: Uuid,
) -> Result<Vec<ValuationLine>, sqlx::Error> {
    sqlx::query_as::<_, ValuationLine>(
        r#"
        SELECT id, snapshot_id, item_id, warehouse_id, location_id,
               quantity_on_hand, unit_cost_minor, total_value_minor, currency
        FROM inventory_valuation_lines
        WHERE snapshot_id = $1
        ORDER BY item_id
        "#,
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await
}
