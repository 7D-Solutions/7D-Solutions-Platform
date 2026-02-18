//! On-hand projection upserts (item_on_hand + item_on_hand_by_status).
//!
//! ## Base-uom contract (CRITICAL)
//!
//! All `quantity_*` parameters accepted by functions in this module MUST be in
//! **base_uom units**. Callers are responsible for canonicalizing input quantities
//! via [`crate::domain::guards::guard_convert_to_base`] (or equivalently
//! [`crate::domain::uom::convert::to_base_uom`]) before calling these functions.
//!
//! Storing non-base quantities here would silently corrupt:
//!   - On-hand availability checks (issue guard reads `quantity_on_hand`)
//!   - FIFO cost allocation (layers and on_hand must agree on unit)
//!   - Any downstream reporting that assumes a single canonical unit per item
//!
//! ## Status bucket contract
//!
//! `available_status_on_hand` in `item_on_hand` is a denormalized cache that
//! mirrors the 'available' row in `item_on_hand_by_status`. Both must be updated
//! atomically within the same transaction.
//!
//! Default status for receipts is 'available'. Only available stock is reservable.
//!
//! ## Location contract
//!
//! `location_id` is optional. When `None`, the projection row is keyed on
//! (tenant_id, item_id, warehouse_id) with location_id IS NULL — identical to
//! the pre-location behavior. When `Some(loc)`, the row is keyed on the 4-tuple
//! (tenant_id, item_id, warehouse_id, location_id), separated from all other
//! locations and from the null-location row.
//!
//! The two partial unique indexes on `item_on_hand` enforce this separation:
//!   - item_on_hand_null_loc  : UNIQUE (tenant, item, warehouse) WHERE location IS NULL
//!   - item_on_hand_with_loc  : UNIQUE (tenant, item, warehouse, location) WHERE location IS NOT NULL
//!
//! ## Usage pattern
//!
//! ```text
//! Guard → Lock → FIFO → Mutation →
//!     upsert_after_receipt + add_to_available_bucket   (receipt path)
//!     upsert_after_issue   + set_available_bucket      (issue path, null location)
//!     decrement_for_issue  + decrement_available_bucket (issue path, with location)
//!     → Outbox
//! ```

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Receipt upsert
// ============================================================================

/// Upsert the `item_on_hand` projection after a stock receipt.
///
/// When `location_id` is `None`, upserts the null-location row (keyed by the
/// partial unique index `item_on_hand_null_loc`). When `Some`, upserts the
/// location-specific row (keyed by `item_on_hand_with_loc`).
///
/// # Invariant
///
/// `quantity_received` **must** be in base_uom units.
pub async fn upsert_after_receipt(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
    // quantity_received: MUST be in base_uom units.
    quantity_received: i64,
    // unit_cost_minor: per-unit cost in minor currency units (e.g. cents).
    unit_cost_minor: i64,
    currency: &str,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    let total_cost_added = quantity_received * unit_cost_minor;

    match location_id {
        None => {
            sqlx::query(
                r#"
                INSERT INTO item_on_hand
                    (tenant_id, item_id, warehouse_id, location_id,
                     quantity_on_hand, available_status_on_hand,
                     total_cost_minor, currency, last_ledger_entry_id, projected_at)
                VALUES ($1, $2, $3, NULL, $4, $4, $5, $6, $7, NOW())
                ON CONFLICT (tenant_id, item_id, warehouse_id)
                WHERE location_id IS NULL
                DO UPDATE
                    SET quantity_on_hand         = item_on_hand.quantity_on_hand + $4,
                        available_status_on_hand = item_on_hand.available_status_on_hand + $4,
                        total_cost_minor         = item_on_hand.total_cost_minor + $5,
                        last_ledger_entry_id     = $7,
                        projected_at             = NOW()
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .bind(quantity_received)
            .bind(total_cost_added)
            .bind(currency)
            .bind(ledger_entry_id)
            .execute(&mut **tx)
            .await?;
        }
        Some(loc_id) => {
            sqlx::query(
                r#"
                INSERT INTO item_on_hand
                    (tenant_id, item_id, warehouse_id, location_id,
                     quantity_on_hand, available_status_on_hand,
                     total_cost_minor, currency, last_ledger_entry_id, projected_at)
                VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8, NOW())
                ON CONFLICT (tenant_id, item_id, warehouse_id, location_id)
                WHERE location_id IS NOT NULL
                DO UPDATE
                    SET quantity_on_hand         = item_on_hand.quantity_on_hand + $5,
                        available_status_on_hand = item_on_hand.available_status_on_hand + $5,
                        total_cost_minor         = item_on_hand.total_cost_minor + $6,
                        last_ledger_entry_id     = $8,
                        projected_at             = NOW()
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .bind(loc_id)
            .bind(quantity_received)
            .bind(total_cost_added)
            .bind(currency)
            .bind(ledger_entry_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

/// Increment the 'available' status bucket after a receipt.
///
/// Creates the row if it does not exist (first receipt); otherwise increments.
/// Must be called in the same transaction as `upsert_after_receipt`.
/// Status buckets are warehouse-level (no location separation in v1).
pub async fn add_to_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    delta_qty: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand_by_status
            (tenant_id, item_id, warehouse_id, status, quantity_on_hand)
        VALUES ($1, $2, $3, 'available', $4)
        ON CONFLICT (tenant_id, item_id, warehouse_id, status) DO UPDATE
            SET quantity_on_hand = item_on_hand_by_status.quantity_on_hand + $4,
                updated_at       = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(delta_qty)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

// ============================================================================
// Issue upserts (null location)
// ============================================================================

/// Upsert the `item_on_hand` projection after a stock issue (null location path).
///
/// Overwrites `quantity_on_hand`, `available_status_on_hand`, and `total_cost_minor`
/// with the caller-computed post-issue values. The caller computes these from FIFO
/// layer sums (holding a `FOR UPDATE` lock) to avoid races.
///
/// # Invariant
///
/// `new_quantity_on_hand` and `post_issue_total_cost` MUST be in base_uom units.
/// They are derived from `sum(layer.quantity_remaining)` after FIFO consumption.
pub async fn upsert_after_issue(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    // new_quantity_on_hand: MUST be in base_uom units.
    new_quantity_on_hand: i64,
    // post_issue_total_cost: remaining total cost in minor currency units.
    post_issue_total_cost: i64,
    currency: &str,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, location_id,
             quantity_on_hand, available_status_on_hand,
             total_cost_minor, currency, last_ledger_entry_id, projected_at)
        VALUES ($1, $2, $3, NULL, $4, $4, $5, $6, $7, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id)
        WHERE location_id IS NULL
        DO UPDATE
            SET quantity_on_hand         = $4,
                available_status_on_hand = $4,
                total_cost_minor         = $5,
                last_ledger_entry_id     = $7,
                projected_at             = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(new_quantity_on_hand)
    .bind(post_issue_total_cost)
    .bind(currency)
    .bind(ledger_entry_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Set the 'available' status bucket to an absolute quantity after an issue (null location).
///
/// Must be called in the same transaction as `upsert_after_issue`.
pub async fn set_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    new_qty: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand_by_status
            (tenant_id, item_id, warehouse_id, status, quantity_on_hand)
        VALUES ($1, $2, $3, 'available', $4)
        ON CONFLICT (tenant_id, item_id, warehouse_id, status) DO UPDATE
            SET quantity_on_hand = $4,
                updated_at       = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(new_qty)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

// ============================================================================
// Issue decrements (location-aware path)
// ============================================================================

/// Decrement the location-specific `item_on_hand` row after an issue.
///
/// Used when `location_id` is `Some`. Applies a delta decrement (not absolute set)
/// since the FIFO layer sum is warehouse-level, not location-level. The location
/// row must already exist (callers verify availability from this row before calling).
///
/// `qty_issued` and `cost_issued` are the amounts consumed from this location.
pub async fn decrement_for_issue(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    qty_issued: i64,
    cost_issued: i64,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE item_on_hand
        SET quantity_on_hand         = GREATEST(0, quantity_on_hand - $4),
            available_status_on_hand = GREATEST(0, available_status_on_hand - $4),
            total_cost_minor         = GREATEST(0, total_cost_minor - $5),
            last_ledger_entry_id     = $6,
            projected_at             = NOW()
        WHERE tenant_id    = $1
          AND item_id      = $2
          AND warehouse_id = $3
          AND location_id  = $7
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(qty_issued)
    .bind(cost_issued)
    .bind(ledger_entry_id)
    .bind(location_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Decrement the warehouse-level 'available' status bucket by delta after a location issue.
///
/// Status buckets are warehouse-level; must stay in sync with total on-hand across
/// all locations. Called in place of `set_available_bucket` when location is specified.
pub async fn decrement_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty_issued: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE item_on_hand_by_status
        SET quantity_on_hand = GREATEST(0, quantity_on_hand - $4),
            updated_at       = NOW()
        WHERE tenant_id    = $1
          AND item_id      = $2
          AND warehouse_id = $3
          AND status       = 'available'
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(qty_issued)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
