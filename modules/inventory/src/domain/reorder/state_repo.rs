//! Low-stock signal dedup state repository.
//!
//! Each row in `inv_low_stock_state` represents the last-known threshold state
//! for a (tenant, item, location) combination.  The evaluator reads and updates
//! these rows inside a transaction alongside the outbox insert so that the
//! dedup guarantee is atomic.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
pub struct LowStockState {
    pub id: Uuid,
    pub below_threshold: bool,
}

// ============================================================================
// Repository
// ============================================================================

/// Ensure a state row exists for (tenant, item, location), then lock it
/// for update within the current transaction.
///
/// If no row exists yet it is inserted with `below_threshold = false`.
/// The returned `LowStockState` reflects the **current** value in the DB.
pub async fn ensure_and_lock(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    location_id: Option<Uuid>,
) -> Result<LowStockState, sqlx::Error> {
    // Step 1: Insert the row if it does not already exist (idempotent).
    match location_id {
        None => {
            sqlx::query(
                r#"
                INSERT INTO inv_low_stock_state (tenant_id, item_id, location_id, below_threshold)
                VALUES ($1, $2, NULL, false)
                ON CONFLICT (tenant_id, item_id) WHERE location_id IS NULL
                DO NOTHING
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .execute(&mut **tx)
            .await?;
        }
        Some(loc_id) => {
            sqlx::query(
                r#"
                INSERT INTO inv_low_stock_state (tenant_id, item_id, location_id, below_threshold)
                VALUES ($1, $2, $3, false)
                ON CONFLICT (tenant_id, item_id, location_id) WHERE location_id IS NOT NULL
                DO NOTHING
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(loc_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    // Step 2: SELECT FOR UPDATE — lock and read the current value.
    let state: LowStockState = match location_id {
        None => {
            sqlx::query_as::<_, LowStockState>(
                r#"
                SELECT id, below_threshold
                FROM inv_low_stock_state
                WHERE tenant_id = $1 AND item_id = $2 AND location_id IS NULL
                FOR UPDATE
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .fetch_one(&mut **tx)
            .await?
        }
        Some(loc_id) => {
            sqlx::query_as::<_, LowStockState>(
                r#"
                SELECT id, below_threshold
                FROM inv_low_stock_state
                WHERE tenant_id = $1 AND item_id = $2 AND location_id = $3
                FOR UPDATE
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(loc_id)
            .fetch_one(&mut **tx)
            .await?
        }
    };

    Ok(state)
}

/// Mark the state as "below threshold" (signal was emitted).
pub async fn set_below(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE inv_low_stock_state
        SET below_threshold = true, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Mark the state as "above threshold" (re-arms for next crossing).
pub async fn set_above(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE inv_low_stock_state
        SET below_threshold = false, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
