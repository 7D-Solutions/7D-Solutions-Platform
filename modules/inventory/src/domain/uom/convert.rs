//! UoM conversion helpers: canonicalize input quantities to base_uom.
//!
//! ## Rounding rule (deterministic, no silent drift)
//!
//! `result = (quantity as f64 * factor).round() as i64`
//!
//! `f64::round()` implements round-half-away-from-zero. This is applied **once**
//! per conversion step — never chained — so the rounding error is bounded to ≤ 0.5
//! base units per operation.
//!
//! ## Single-hop only
//!
//! Only direct (from_uom → base_uom) conversions are supported. Multi-hop chains
//! are rejected: each hop accumulates ≤ 0.5 unit error, and chaining makes auditing
//! harder without meaningful benefit for item-level UoMs.
//!
//! ## Identity
//!
//! If `from_uom_id == base_uom_id`, `quantity` is returned unchanged without
//! consulting the conversion table.

use thiserror::Error;
use uuid::Uuid;

use crate::domain::uom::models::ItemUomConversion;

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error, PartialEq)]
pub enum ConvertError {
    #[error(
        "UoM '{from}' is not directly convertible to base UoM '{to}': \
         no conversion registered for this item in that direction"
    )]
    ConversionNotFound { from: Uuid, to: Uuid },

    #[error(
        "Conversion of {quantity} units with factor {factor:.6} rounds to zero; \
         minimum 1 base unit required"
    )]
    QuantityRoundsToZero { quantity: i64, factor: f64 },
}

// ============================================================================
// Public API
// ============================================================================

/// Convert `quantity` from `from_uom_id` to `base_uom_id` using the item's
/// registered conversion table.
///
/// # Behaviour
///
/// | Condition                         | Result                              |
/// |-----------------------------------|-------------------------------------|
/// | `from_uom_id == base_uom_id`      | identity — `quantity` returned as-is |
/// | Direct conversion found           | `round(quantity × factor)` returned |
/// | No direct conversion found        | `ConvertError::ConversionNotFound`  |
/// | Result rounds to 0                | `ConvertError::QuantityRoundsToZero`|
///
/// # Rounding
///
/// `f64::round()` (round-half-away-from-zero) is used exactly once. This is the
/// canonical rounding rule for all inventory UoM conversions in this system.
pub fn to_base_uom(
    quantity: i64,
    from_uom_id: Uuid,
    base_uom_id: Uuid,
    conversions: &[ItemUomConversion],
) -> Result<i64, ConvertError> {
    // Identity: quantity is already in base units.
    if from_uom_id == base_uom_id {
        return Ok(quantity);
    }

    // Find a direct conversion from_uom → base_uom.
    let factor = conversions
        .iter()
        .find(|c| c.from_uom_id == from_uom_id && c.to_uom_id == base_uom_id)
        .map(|c| c.factor)
        .ok_or(ConvertError::ConversionNotFound {
            from: from_uom_id,
            to: base_uom_id,
        })?;

    apply_factor(quantity, factor)
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Apply a conversion factor to `quantity`.
///
/// Rounding: `(quantity as f64 * factor).round() as i64`.
/// Returns an error if the result rounds to zero.
fn apply_factor(quantity: i64, factor: f64) -> Result<i64, ConvertError> {
    let result = (quantity as f64 * factor).round() as i64;
    if result == 0 {
        return Err(ConvertError::QuantityRoundsToZero { quantity, factor });
    }
    Ok(result)
}

// ============================================================================
// Unit tests (pure, no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn conv(from: Uuid, to: Uuid, factor: f64) -> ItemUomConversion {
        ItemUomConversion {
            id: Uuid::new_v4(),
            tenant_id: "t1".into(),
            item_id: Uuid::new_v4(),
            from_uom_id: from,
            to_uom_id: to,
            factor,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn identity_returns_unchanged() {
        let id = Uuid::new_v4();
        assert_eq!(to_base_uom(10, id, id, &[]), Ok(10));
        assert_eq!(to_base_uom(0, id, id, &[]), Ok(0));
    }

    #[test]
    fn direct_conversion_whole_factor() {
        let ea = Uuid::new_v4();
        let bx = Uuid::new_v4();
        // 1 box = 12 ea
        let c = conv(bx, ea, 12.0);
        assert_eq!(to_base_uom(3, bx, ea, &[c]), Ok(36));
        assert_eq!(to_base_uom(1, bx, ea, &[conv(bx, ea, 12.0)]), Ok(12));
    }

    #[test]
    fn fractional_factor_rounds_half_away_from_zero() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        // 3 × 0.5 = 1.5 → round to 2
        assert_eq!(to_base_uom(3, from, to, &[conv(from, to, 0.5)]), Ok(2));
        // 1 × 0.5 = 0.5 → round to 1
        assert_eq!(to_base_uom(1, from, to, &[conv(from, to, 0.5)]), Ok(1));
        // 2 × 0.5 = 1.0 → exact
        assert_eq!(to_base_uom(2, from, to, &[conv(from, to, 0.5)]), Ok(1));
    }

    #[test]
    fn conversion_not_found_for_unknown_pair() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        let other = Uuid::new_v4();
        // conversion table only has from→other, not from→to
        assert!(matches!(
            to_base_uom(1, from, to, &[conv(from, other, 2.0)]),
            Err(ConvertError::ConversionNotFound { .. })
        ));
    }

    #[test]
    fn wrong_direction_not_found() {
        let ea = Uuid::new_v4();
        let bx = Uuid::new_v4();
        // Only box→ea registered; ea→box is not the reverse
        let c = conv(bx, ea, 12.0);
        assert!(matches!(
            to_base_uom(1, ea, bx, &[c]),
            Err(ConvertError::ConversionNotFound { .. })
        ));
    }

    #[test]
    fn rounds_to_zero_rejected() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        // 1 × 0.1 = 0.1 → rounds to 0
        assert!(matches!(
            to_base_uom(1, from, to, &[conv(from, to, 0.1)]),
            Err(ConvertError::QuantityRoundsToZero { .. })
        ));
    }

    #[test]
    fn large_quantity_conversion() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        // 1000 pallets × 48 ea/pallet = 48,000 ea
        assert_eq!(
            to_base_uom(1000, from, to, &[conv(from, to, 48.0)]),
            Ok(48_000)
        );
    }

    #[test]
    fn no_conversions_in_table_returns_not_found() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        assert!(matches!(
            to_base_uom(5, from, to, &[]),
            Err(ConvertError::ConversionNotFound { .. })
        ));
    }
}
