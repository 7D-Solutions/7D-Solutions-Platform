//! Pure FIFO cost-layer consumption algorithm.
//!
//! Given a list of available layers (sorted oldest-first by the caller) and a
//! quantity to consume, returns the ConsumedLayer records that sum exactly to
//! the requested quantity.
//!
//! This is a pure function — no database access.
//! The caller is responsible for:
//!   1. Fetching layers in correct FIFO order (received_at ASC, ledger_entry_id ASC).
//!   2. Locking them (SELECT … FOR UPDATE) before calling.
//!   3. Applying the returned consumptions to the database.
//!
//! Invariant: sum(ConsumedLayer.extended_cost_minor) == total_cost_minor used in the event.

use thiserror::Error;
use uuid::Uuid;

use crate::events::contracts::ConsumedLayer;

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum FifoError {
    #[error("Insufficient stock: requested {requested}, available {available}")]
    InsufficientQuantity { requested: i64, available: i64 },
}

// ============================================================================
// Input type
// ============================================================================

/// A single available FIFO layer as read from the database.
#[derive(Debug, Clone)]
pub struct AvailableLayer {
    pub layer_id: Uuid,
    pub quantity_remaining: i64,
    pub unit_cost_minor: i64,
}

// ============================================================================
// Algorithm
// ============================================================================

/// Consume `quantity` units from `layers` in FIFO order.
///
/// Layers MUST be sorted oldest-first by the caller.
/// Returns a vec of ConsumedLayer records — one entry per partially or fully
/// consumed layer.  Each `extended_cost_minor` is precomputed as
/// `quantity × unit_cost_minor` to avoid floating-point arithmetic.
pub fn consume_fifo(
    layers: &[AvailableLayer],
    quantity: i64,
) -> Result<Vec<ConsumedLayer>, FifoError> {
    let total_available: i64 = layers.iter().map(|l| l.quantity_remaining).sum();

    if total_available < quantity {
        return Err(FifoError::InsufficientQuantity {
            requested: quantity,
            available: total_available,
        });
    }

    let mut result = Vec::new();
    let mut remaining = quantity;

    for layer in layers {
        if remaining <= 0 {
            break;
        }
        let take = remaining.min(layer.quantity_remaining);
        result.push(ConsumedLayer {
            layer_id: layer.layer_id,
            quantity: take,
            unit_cost_minor: layer.unit_cost_minor,
            extended_cost_minor: take * layer.unit_cost_minor,
        });
        remaining -= take;
    }

    Ok(result)
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(qty: i64, cost: i64) -> AvailableLayer {
        AvailableLayer {
            layer_id: Uuid::new_v4(),
            quantity_remaining: qty,
            unit_cost_minor: cost,
        }
    }

    #[test]
    fn consume_single_layer_exact() {
        let layers = vec![layer(10, 500)];
        let consumed = consume_fifo(&layers, 10).unwrap();
        assert_eq!(consumed.len(), 1);
        assert_eq!(consumed[0].quantity, 10);
        assert_eq!(consumed[0].extended_cost_minor, 5000);
    }

    #[test]
    fn consume_partial_single_layer() {
        let layers = vec![layer(10, 500)];
        let consumed = consume_fifo(&layers, 3).unwrap();
        assert_eq!(consumed.len(), 1);
        assert_eq!(consumed[0].quantity, 3);
        assert_eq!(consumed[0].extended_cost_minor, 1500);
    }

    #[test]
    fn consume_across_two_layers() {
        let layers = vec![layer(5, 500), layer(10, 800)];
        let consumed = consume_fifo(&layers, 8).unwrap();
        assert_eq!(consumed.len(), 2);
        assert_eq!(consumed[0].quantity, 5);
        assert_eq!(consumed[0].extended_cost_minor, 2500);
        assert_eq!(consumed[1].quantity, 3);
        assert_eq!(consumed[1].extended_cost_minor, 2400);
    }

    #[test]
    fn consume_exact_across_three_layers() {
        let layers = vec![layer(4, 100), layer(6, 200), layer(10, 300)];
        let consumed = consume_fifo(&layers, 10).unwrap();
        assert_eq!(consumed.len(), 2);
        let total: i64 = consumed.iter().map(|c| c.extended_cost_minor).sum();
        assert_eq!(total, 4 * 100 + 6 * 200);
    }

    #[test]
    fn total_cost_equals_sum_of_extended_costs() {
        let layers = vec![layer(3, 100), layer(7, 200), layer(10, 300)];
        let consumed = consume_fifo(&layers, 15).unwrap();
        let sum: i64 = consumed.iter().map(|c| c.extended_cost_minor).sum();
        // 3*100 + 7*200 + 5*300 = 300 + 1400 + 1500 = 3200
        assert_eq!(sum, 3200);
    }

    #[test]
    fn insufficient_quantity_returns_error() {
        let layers = vec![layer(3, 100)];
        let err = consume_fifo(&layers, 10).unwrap_err();
        assert!(matches!(
            err,
            FifoError::InsufficientQuantity {
                requested: 10,
                available: 3
            }
        ));
    }

    #[test]
    fn empty_layers_with_nonzero_demand_errors() {
        let err = consume_fifo(&[], 1).unwrap_err();
        assert!(matches!(
            err,
            FifoError::InsufficientQuantity {
                requested: 1,
                available: 0
            }
        ));
    }

    #[test]
    fn fifo_respects_layer_order() {
        // First layer has cost 1000; second has cost 500.
        // Consuming 3 should draw from first (oldest) layer entirely.
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let layers = vec![
            AvailableLayer { layer_id: first_id, quantity_remaining: 3, unit_cost_minor: 1000 },
            AvailableLayer { layer_id: second_id, quantity_remaining: 5, unit_cost_minor: 500 },
        ];
        let consumed = consume_fifo(&layers, 3).unwrap();
        assert_eq!(consumed.len(), 1);
        assert_eq!(consumed[0].layer_id, first_id);
        assert_eq!(consumed[0].extended_cost_minor, 3000);
    }
}
