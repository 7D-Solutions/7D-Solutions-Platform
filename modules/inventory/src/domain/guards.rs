//! Guard checks for inventory operations.
//!
//! Each guard is a pure function (no DB) or a simple DB lookup that validates a
//! precondition. Guards always run BEFORE any mutation.
//!
//! Pattern: Guard → Mutation → Outbox atomicity.

use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::items::{Item, TrackingMode};

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("Item not found")]
    ItemNotFound,

    #[error("Item is inactive and cannot receive stock")]
    ItemInactive,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Guards
// ============================================================================

/// Guard: item must exist and be active for the given tenant.
///
/// Returns the item on success so callers can use its fields (e.g. sku)
/// without a second query.
pub async fn guard_item_active(
    pool: &PgPool,
    item_id: Uuid,
    tenant_id: &str,
) -> Result<Item, GuardError> {
    let item = sqlx::query_as::<_, Item>(
        "SELECT * FROM items WHERE id = $1 AND tenant_id = $2",
    )
    .bind(item_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(GuardError::ItemNotFound)?;

    if !item.active {
        return Err(GuardError::ItemInactive);
    }

    Ok(item)
}

/// Guard: quantity must be strictly positive.
pub fn guard_quantity_positive(quantity: i64) -> Result<(), GuardError> {
    if quantity <= 0 {
        return Err(GuardError::Validation(
            "quantity must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

/// Guard: unit cost must be present and positive for stock items.
///
/// All inventory items are stock items; every receipt requires a positive cost.
pub fn guard_cost_present(unit_cost_minor: i64) -> Result<(), GuardError> {
    if unit_cost_minor <= 0 {
        return Err(GuardError::Validation(
            "unit_cost_minor must be greater than zero for stock items".to_string(),
        ));
    }
    Ok(())
}

/// Guard: serial-tracked items must move in positive integer units.
///
/// Since quantity is already i64, integer is guaranteed. This guard enforces
/// that serial-tracked items have quantity > 0, which is required for
/// deterministic serial number assignment in downstream beads.
///
/// For none/lot items this guard is a no-op; `guard_quantity_positive` handles
/// the general positive-quantity requirement.
pub fn guard_serial_quantity(tracking_mode: TrackingMode, quantity: i64) -> Result<(), GuardError> {
    if tracking_mode == TrackingMode::Serial && quantity <= 0 {
        return Err(GuardError::Validation(
            "serial-tracked items must move in positive integer units (quantity > 0)".to_string(),
        ));
    }
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantity_positive_rejects_zero() {
        assert!(matches!(
            guard_quantity_positive(0),
            Err(GuardError::Validation(_))
        ));
    }

    #[test]
    fn quantity_positive_rejects_negative() {
        assert!(matches!(
            guard_quantity_positive(-1),
            Err(GuardError::Validation(_))
        ));
    }

    #[test]
    fn quantity_positive_accepts_positive() {
        assert!(guard_quantity_positive(1).is_ok());
        assert!(guard_quantity_positive(1_000_000).is_ok());
    }

    #[test]
    fn cost_present_rejects_zero() {
        assert!(matches!(
            guard_cost_present(0),
            Err(GuardError::Validation(_))
        ));
    }

    #[test]
    fn cost_present_rejects_negative() {
        assert!(matches!(
            guard_cost_present(-100),
            Err(GuardError::Validation(_))
        ));
    }

    #[test]
    fn cost_present_accepts_positive() {
        assert!(guard_cost_present(1).is_ok());
        assert!(guard_cost_present(50_000).is_ok());
    }

    #[test]
    fn serial_quantity_rejects_zero_for_serial() {
        assert!(matches!(
            guard_serial_quantity(TrackingMode::Serial, 0),
            Err(GuardError::Validation(_))
        ));
    }

    #[test]
    fn serial_quantity_rejects_negative_for_serial() {
        assert!(matches!(
            guard_serial_quantity(TrackingMode::Serial, -5),
            Err(GuardError::Validation(_))
        ));
    }

    #[test]
    fn serial_quantity_accepts_positive_for_serial() {
        assert!(guard_serial_quantity(TrackingMode::Serial, 1).is_ok());
        assert!(guard_serial_quantity(TrackingMode::Serial, 100).is_ok());
    }

    #[test]
    fn serial_quantity_noop_for_none_and_lot() {
        // For none/lot items, guard_quantity_positive handles the requirement.
        // guard_serial_quantity is a no-op so callers can apply it unconditionally.
        assert!(guard_serial_quantity(TrackingMode::None, 0).is_ok());
        assert!(guard_serial_quantity(TrackingMode::None, -1).is_ok());
        assert!(guard_serial_quantity(TrackingMode::Lot, 0).is_ok());
        assert!(guard_serial_quantity(TrackingMode::Lot, -1).is_ok());
    }
}
