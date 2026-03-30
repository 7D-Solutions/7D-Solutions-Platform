//! Valuation snapshot domain models.
//!
//! These are read-only reporting types populated by the valuation snapshot
//! builder (bd-2k0i). They represent a point-in-time view of inventory value
//! derived from remaining FIFO layers — they do not affect GL.
//!
//! Value computation:
//!   total_value_minor = sum over remaining layers of (qty_remaining * unit_cost_minor)
//!   unit_cost_minor on a line is the weighted-average of those layers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Snapshot header
// ============================================================================

/// A point-in-time valuation roll-up for a tenant's warehouse (or location).
///
/// `total_value_minor` is the pre-computed sum of all associated
/// `ValuationLine.total_value_minor` entries.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ValuationSnapshot {
    pub id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    /// `None` = warehouse-level roll-up (all locations combined).
    pub location_id: Option<Uuid>,
    /// The instant at which FIFO layers were evaluated.
    pub as_of: DateTime<Utc>,
    /// Sum of all line `total_value_minor` values, in minor currency units.
    pub total_value_minor: i64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Per-item valuation line
// ============================================================================

/// One line under a `ValuationSnapshot` — value for a single item/location.
///
/// `total_value_minor = quantity_on_hand * unit_cost_minor` (pre-computed).
/// `unit_cost_minor` is the weighted-average unit cost of remaining FIFO layers.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ValuationLine {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// `None` when not tracking at location level for this snapshot.
    pub location_id: Option<Uuid>,
    /// Remaining on-hand quantity at `as_of` (sum of layer `quantity_remaining`).
    pub quantity_on_hand: i64,
    /// Weighted-average unit cost across remaining FIFO layers at `as_of`.
    pub unit_cost_minor: i64,
    /// `quantity_on_hand * unit_cost_minor`, in minor currency units.
    pub total_value_minor: i64,
    pub currency: String,
}

// ============================================================================
// Unit tests (pure; DB tests live in integration suite)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the total_value_minor invariant holds for a manually-built line.
    #[test]
    fn total_value_minor_is_qty_times_cost() {
        let qty: i64 = 50;
        let unit_cost: i64 = 1500; // e.g. $15.00
        let expected_total = qty * unit_cost; // 75_000

        // Simulate what the builder would write
        let line = ValuationLine {
            id: Uuid::new_v4(),
            snapshot_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity_on_hand: qty,
            unit_cost_minor: unit_cost,
            total_value_minor: qty * unit_cost,
            currency: "usd".to_string(),
        };

        assert_eq!(line.total_value_minor, expected_total);
    }

    /// Verify that snapshot total equals sum of line totals.
    #[test]
    fn snapshot_total_equals_sum_of_lines() {
        let snapshot_id = Uuid::new_v4();
        let warehouse_id = Uuid::new_v4();
        let now = Utc::now();

        let lines = vec![
            ValuationLine {
                id: Uuid::new_v4(),
                snapshot_id,
                item_id: Uuid::new_v4(),
                warehouse_id,
                location_id: None,
                quantity_on_hand: 10,
                unit_cost_minor: 500,
                total_value_minor: 5_000,
                currency: "usd".to_string(),
            },
            ValuationLine {
                id: Uuid::new_v4(),
                snapshot_id,
                item_id: Uuid::new_v4(),
                warehouse_id,
                location_id: None,
                quantity_on_hand: 20,
                unit_cost_minor: 1_000,
                total_value_minor: 20_000,
                currency: "usd".to_string(),
            },
        ];

        let expected_total: i64 = lines.iter().map(|l| l.total_value_minor).sum();

        let snapshot = ValuationSnapshot {
            id: snapshot_id,
            tenant_id: "t1".to_string(),
            warehouse_id,
            location_id: None,
            as_of: now,
            total_value_minor: expected_total,
            currency: "usd".to_string(),
            created_at: now,
        };

        assert_eq!(snapshot.total_value_minor, 25_000);
    }

    /// location_id = None means warehouse-level roll-up.
    #[test]
    fn location_id_none_is_warehouse_level() {
        let snapshot = ValuationSnapshot {
            id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            as_of: Utc::now(),
            total_value_minor: 0,
            currency: "usd".to_string(),
            created_at: Utc::now(),
        };
        assert!(snapshot.location_id.is_none());
    }
}
