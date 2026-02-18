//! On-hand projection upserts.
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
//! ## Usage pattern
//!
//! These functions run inside the write-path transaction:
//!
//! ```text
//! Guard (validate + convert to base) → Lock → FIFO → Mutation →
//!     upsert_after_receipt / upsert_after_issue → Outbox
//! ```

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Receipt upsert
// ============================================================================

/// Upsert the `item_on_hand` projection after a stock receipt.
///
/// Increments `quantity_on_hand` and `total_cost_minor` by the received amounts.
/// On first receipt for an (tenant, item, warehouse), inserts a new row.
///
/// # Invariant
///
/// `quantity_received` **must** be in base_uom units.
/// `total_cost_added` = `quantity_received × unit_cost_minor`, also in base units.
pub async fn upsert_after_receipt(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    // quantity_received: MUST be in base_uom units.
    quantity_received: i64,
    // unit_cost_minor: per-unit cost in minor currency units (e.g. cents).
    unit_cost_minor: i64,
    currency: &str,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    let total_cost_added = quantity_received * unit_cost_minor;

    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, quantity_on_hand,
             total_cost_minor, currency, last_ledger_entry_id, projected_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) DO UPDATE
            SET quantity_on_hand     = item_on_hand.quantity_on_hand + $4,
                total_cost_minor     = item_on_hand.total_cost_minor + $5,
                last_ledger_entry_id = $7,
                projected_at         = NOW()
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

    Ok(())
}

// ============================================================================
// Issue upsert
// ============================================================================

/// Upsert the `item_on_hand` projection after a stock issue.
///
/// Overwrites `quantity_on_hand` and `total_cost_minor` with the caller-computed
/// post-issue values. The caller must compute these from FIFO layer sums
/// (holding a `FOR UPDATE` lock) to avoid races.
///
/// # Invariant
///
/// `new_quantity_on_hand` and `post_issue_total_cost` MUST be in base_uom units.
/// They are derived from `sum(layer.quantity_remaining)` after FIFO consumption,
/// which is always in base_uom (layers are written in base_uom by the receipt path).
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
            (tenant_id, item_id, warehouse_id, quantity_on_hand,
             total_cost_minor, currency, last_ledger_entry_id, projected_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) DO UPDATE
            SET quantity_on_hand     = $4,
                total_cost_minor     = $5,
                last_ledger_entry_id = $7,
                projected_at         = NOW()
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
