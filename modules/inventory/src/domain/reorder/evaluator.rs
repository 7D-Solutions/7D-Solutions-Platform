//! Low-stock threshold evaluator.
//!
//! Called (best-effort) after every issue and adjustment.  Checks whether
//! the item's available stock has crossed below the configured reorder_point
//! and, if so, emits a deduped `inventory.low_stock_triggered` outbox event.
//!
//! ## Dedup rule (per policy)
//! - available < reorder_point  AND state.below_threshold = false
//!   → emit signal, set state.below_threshold = true
//! - available >= reorder_point AND state.below_threshold = true
//!   → reset state.below_threshold = false  (re-arms for next crossing)
//! - any other combination → no-op
//!
//! ## Policy lookup
//! Given (tenant, item, warehouse, location_id):
//! 1. If location_id is Some(L): check location-specific policy for L.
//! 2. Always: check global (null-location) policy for the item.
//!
//! ## Available qty
//!   Uses `quantity_available` from `item_on_hand` which is generated as
//!   `available_status_on_hand − quantity_reserved`.
//!
//! ## Error handling
//!   Returns `Ok(())` on success.  Callers ignore the result so a failure
//!   here never rolls back the parent issue/adjustment.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::reorder::{models::ReorderPolicy, state_repo},
    events::{
        low_stock_triggered::{
            build_low_stock_triggered_envelope, LowStockTriggeredPayload,
            EVENT_TYPE_LOW_STOCK_TRIGGERED,
        },
    },
};

// ============================================================================
// Public API
// ============================================================================

/// Evaluate low-stock threshold after a stock-reducing mutation.
///
/// * `location_id` — the location where the mutation happened (None = no location).
pub async fn evaluate_low_stock(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
    correlation_id: &str,
    causation_id: Option<String>,
) -> Result<(), sqlx::Error> {
    // --- Collect applicable policies ---
    let mut policies: Vec<ReorderPolicy> = Vec::new();

    if let Some(loc_id) = location_id {
        if let Some(p) = find_location_policy(pool, tenant_id, item_id, loc_id).await? {
            policies.push(p);
        }
    }
    if let Some(p) = find_global_policy(pool, tenant_id, item_id).await? {
        policies.push(p);
    }

    if policies.is_empty() {
        return Ok(());
    }

    // --- Get available qty at the mutation's location scope ---
    let available_qty = get_available_qty(pool, tenant_id, item_id, warehouse_id, location_id).await?;

    // --- Evaluate each policy in its own transaction ---
    for policy in &policies {
        maybe_emit_signal(
            pool,
            tenant_id,
            item_id,
            warehouse_id,
            policy.location_id,
            available_qty,
            policy.reorder_point,
            correlation_id,
            causation_id.clone(),
        )
        .await?;
    }

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

async fn find_location_policy(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    location_id: Uuid,
) -> Result<Option<ReorderPolicy>, sqlx::Error> {
    sqlx::query_as::<_, ReorderPolicy>(
        r#"
        SELECT * FROM reorder_policies
        WHERE tenant_id = $1 AND item_id = $2 AND location_id = $3
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(location_id)
    .fetch_optional(pool)
    .await
}

async fn find_global_policy(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Option<ReorderPolicy>, sqlx::Error> {
    sqlx::query_as::<_, ReorderPolicy>(
        r#"
        SELECT * FROM reorder_policies
        WHERE tenant_id = $1 AND item_id = $2 AND location_id IS NULL
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_optional(pool)
    .await
}

/// Read the `quantity_available` generated column from item_on_hand.
/// Returns 0 if no row exists yet.
async fn get_available_qty(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
) -> Result<i64, sqlx::Error> {
    let qty: Option<i64> = match location_id {
        None => {
            sqlx::query_scalar(
                r#"
                SELECT quantity_available
                FROM item_on_hand
                WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                  AND location_id IS NULL
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .fetch_optional(pool)
            .await?
        }
        Some(loc_id) => {
            sqlx::query_scalar(
                r#"
                SELECT quantity_available
                FROM item_on_hand
                WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                  AND location_id = $4
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .bind(loc_id)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(qty.unwrap_or(0))
}

/// Evaluate a single policy and emit the outbox event if needed.
/// All state mutations and outbox inserts are in one atomic transaction.
#[allow(clippy::too_many_arguments)]
async fn maybe_emit_signal(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    policy_location_id: Option<Uuid>,
    available_qty: i64,
    reorder_point: i64,
    correlation_id: &str,
    causation_id: Option<String>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let state = state_repo::ensure_and_lock(&mut tx, tenant_id, item_id, policy_location_id).await?;

    let crossing_below = available_qty < reorder_point && !state.below_threshold;
    let recovering_above = available_qty >= reorder_point && state.below_threshold;

    if crossing_below {
        // Update state first
        state_repo::set_below(&mut tx, state.id).await?;

        // Build and insert outbox event
        let event_id = Uuid::new_v4();
        let triggered_at = Utc::now();
        let payload = LowStockTriggeredPayload {
            tenant_id: tenant_id.to_string(),
            item_id,
            warehouse_id,
            location_id: policy_location_id,
            reorder_point,
            available_qty,
            triggered_at,
        };
        let envelope = build_low_stock_triggered_envelope(
            event_id,
            tenant_id.to_string(),
            correlation_id.to_string(),
            causation_id.clone(),
            payload,
        );
        let envelope_json = serde_json::to_string(&envelope)
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO inv_outbox
                (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
                 payload, correlation_id, causation_id, schema_version)
            VALUES
                ($1, $2, 'inventory_item', $3, $4, $5::JSONB, $6, $7, '1.0.0')
            "#,
        )
        .bind(event_id)
        .bind(EVENT_TYPE_LOW_STOCK_TRIGGERED)
        .bind(item_id.to_string())
        .bind(tenant_id)
        .bind(&envelope_json)
        .bind(correlation_id)
        .bind(causation_id)
        .execute(&mut *tx)
        .await?;
    } else if recovering_above {
        state_repo::set_above(&mut tx, state.id).await?;
    }
    // else: no state change needed

    tx.commit().await?;
    Ok(())
}
