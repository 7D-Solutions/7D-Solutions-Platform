//! Inventory invariant checks.
//!
//! Invariants:
//! 1. on_hand_matches_ledger: item_on_hand.quantity_on_hand must equal
//!    SUM(inventory_ledger.quantity) for each (tenant_id, item_id, warehouse_id).
//!    The on-hand projection is derived from the append-only ledger; drift indicates
//!    a missed projection rebuild or a direct table update bypassing the write path.
//! 2. no_negative_on_hand: For non-lot-tracked items (tracking_mode = 'none'),
//!    item_on_hand.quantity_on_hand must be >= 0. Physical stock cannot be negative.
//!
//! SQL forms (for manual verification):
//! ```sql
//! -- Invariant 1: on_hand_matches_ledger
//! SELECT ioh.tenant_id, ioh.item_id, ioh.warehouse_id,
//!        ioh.quantity_on_hand AS projection,
//!        COALESCE(SUM(il.quantity), 0) AS ledger_sum
//! FROM item_on_hand ioh
//! LEFT JOIN inventory_ledger il
//!        ON il.tenant_id = ioh.tenant_id
//!       AND il.item_id   = ioh.item_id
//!       AND il.warehouse_id = ioh.warehouse_id
//! GROUP BY ioh.tenant_id, ioh.item_id, ioh.warehouse_id, ioh.quantity_on_hand
//! HAVING ioh.quantity_on_hand <> COALESCE(SUM(il.quantity), 0);
//!
//! -- Invariant 2: no_negative_on_hand
//! SELECT ioh.tenant_id, ioh.item_id, ioh.warehouse_id, ioh.quantity_on_hand
//! FROM item_on_hand ioh
//! JOIN items i ON i.id = ioh.item_id AND i.tenant_id = ioh.tenant_id
//! WHERE i.tracking_mode = 'none'
//!   AND ioh.quantity_on_hand < 0;
//! ```

use anyhow::Result;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use super::Violation;

const MODULE: &str = "inventory";

/// Run all inventory invariant checks. Returns list of violations found.
pub async fn run_checks(pool: &PgPool) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();

    info!("Inventory: checking on_hand_matches_ledger invariant");
    violations.extend(check_on_hand_matches_ledger(pool).await?);

    info!("Inventory: checking no_negative_on_hand invariant");
    violations.extend(check_no_negative_on_hand(pool).await?);

    Ok(violations)
}

/// Invariant 1: item_on_hand projection matches the sum of ledger entries.
///
/// The inventory ledger is append-only and authoritative. The item_on_hand table
/// is a materialised projection rebuilt from the ledger. If they diverge, either
/// the projection was not updated (event consumer fell behind or crashed mid-rebuild)
/// or a direct write bypassed the write path.
///
/// Ledger quantities are signed: positive = stock in, negative = stock out.
/// The sum of all ledger rows for a (tenant, item, warehouse) must equal
/// item_on_hand.quantity_on_hand.
async fn check_on_hand_matches_ledger(pool: &PgPool) -> Result<Vec<Violation>> {
    // Note: item_on_hand may have location_id (nullable). We group by the full key
    // but the ledger join uses (tenant_id, item_id, warehouse_id) only.
    // This intentionally compares across all locations to catch aggregate drift.
    let rows: Vec<(String, Uuid, Uuid, i64, i64)> = sqlx::query_as(
        r#"
        SELECT ioh.tenant_id,
               ioh.item_id,
               ioh.warehouse_id,
               SUM(ioh.quantity_on_hand)::BIGINT AS projection_total,
               COALESCE(ledger.qty_sum, 0)::BIGINT AS ledger_total
        FROM item_on_hand ioh
        LEFT JOIN (
            SELECT tenant_id, item_id, warehouse_id,
                   SUM(quantity) AS qty_sum
            FROM inventory_ledger
            GROUP BY tenant_id, item_id, warehouse_id
        ) ledger ON ledger.tenant_id = ioh.tenant_id
               AND ledger.item_id    = ioh.item_id
               AND ledger.warehouse_id = ioh.warehouse_id
        GROUP BY ioh.tenant_id, ioh.item_id, ioh.warehouse_id, ledger.qty_sum
        HAVING SUM(ioh.quantity_on_hand) <> COALESCE(ledger.qty_sum, 0)
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (tenant_id, item_id, warehouse_id, projection, ledger) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "on_hand_matches_ledger",
        rows.len() as i64,
        format!(
            "first violation: tenant={tenant_id} item={item_id} warehouse={warehouse_id} projection={projection} ledger={ledger}"
        ),
    )])
}

/// Invariant 2: Non-lot-tracked items cannot have negative on-hand quantity.
///
/// For items with tracking_mode='none', physical stock is a simple counter.
/// A negative quantity_on_hand means more stock was issued than was ever received,
/// which is physically impossible and indicates a ledger corruption or
/// a missing receipt entry.
///
/// Lot-tracked and serial-tracked items are excluded because their negative
/// balance semantics are governed separately by lot lifecycle rules.
async fn check_no_negative_on_hand(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(String, Uuid, Uuid, i64)> = sqlx::query_as(
        r#"
        SELECT ioh.tenant_id,
               ioh.item_id,
               ioh.warehouse_id,
               ioh.quantity_on_hand::BIGINT
        FROM item_on_hand ioh
        JOIN items i ON i.id = ioh.item_id AND i.tenant_id = ioh.tenant_id
        WHERE i.tracking_mode = 'none'
          AND ioh.quantity_on_hand < 0
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (tenant_id, item_id, warehouse_id, qty) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "no_negative_on_hand",
        rows.len() as i64,
        format!(
            "first violation: tenant={tenant_id} item={item_id} warehouse={warehouse_id} quantity_on_hand={qty}"
        ),
    )])
}
