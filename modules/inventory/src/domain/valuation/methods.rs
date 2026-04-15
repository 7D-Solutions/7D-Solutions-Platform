//! Pure valuation algorithms: FIFO, LIFO, WAC, Standard Cost.
//!
//! Each function takes a set of receipt layers for a single item and produces
//! a deterministic valuation result. No database access — the caller provides
//! the layer data and applies the result.
//!
//! All arithmetic uses i64 minor currency units to avoid floating-point drift.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Valuation method enum
// ============================================================================

/// The four supported inventory valuation methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValuationMethod {
    Fifo,
    Lifo,
    Wac,
    StandardCost,
}

impl ValuationMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fifo => "fifo",
            Self::Lifo => "lifo",
            Self::Wac => "wac",
            Self::StandardCost => "standard_cost",
        }
    }
}

impl std::fmt::Display for ValuationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ValuationMethod {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "fifo" => Ok(Self::Fifo),
            "lifo" => Ok(Self::Lifo),
            "wac" => Ok(Self::Wac),
            "standard_cost" => Ok(Self::StandardCost),
            other => Err(format!(
                "invalid valuation method '{}': expected fifo|lifo|wac|standard_cost",
                other
            )),
        }
    }
}

// ============================================================================
// Input types
// ============================================================================

/// A receipt layer with full history (both received and consumed quantities).
///
/// Sorted by `received_at` ASC (oldest first) by the caller.
#[derive(Debug, Clone)]
pub struct FullLayer {
    pub item_id: Uuid,
    pub unit_cost_minor: i64,
    pub quantity_received: i64,
    /// How much of this layer was consumed as of the valuation date.
    pub qty_consumed_at_as_of: i64,
}

impl FullLayer {
    /// Quantity remaining in this layer at the valuation date.
    pub fn qty_remaining(&self) -> i64 {
        self.quantity_received - self.qty_consumed_at_as_of
    }
}

// ============================================================================
// Output type
// ============================================================================

/// Valuation result for a single item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemValuation {
    pub item_id: Uuid,
    pub quantity_on_hand: i64,
    pub unit_cost_minor: i64,
    pub total_value_minor: i64,
    /// Variance from standard cost; 0 for non-standard methods.
    pub variance_minor: i64,
}

// ============================================================================
// FIFO valuation
// ============================================================================

/// Value remaining inventory using FIFO.
///
/// Under FIFO, the oldest layers are consumed first, so remaining inventory
/// carries the cost of the most recent purchases. The value equals the sum
/// of (remaining_qty * unit_cost) across all layers with remaining stock.
///
/// Layers must be for a single item, sorted oldest-first.
pub fn value_fifo(layers: &[FullLayer]) -> Option<ItemValuation> {
    if layers.is_empty() {
        return None;
    }

    let item_id = layers[0].item_id;
    let mut total_qty: i64 = 0;
    let mut total_cost: i64 = 0;

    for layer in layers {
        let remaining = layer.qty_remaining();
        if remaining > 0 {
            total_qty += remaining;
            total_cost += remaining * layer.unit_cost_minor;
        }
    }

    if total_qty == 0 {
        return None;
    }

    let unit_cost = total_cost / total_qty;

    Some(ItemValuation {
        item_id,
        quantity_on_hand: total_qty,
        unit_cost_minor: unit_cost,
        total_value_minor: total_cost,
        variance_minor: 0,
    })
}

// ============================================================================
// LIFO valuation
// ============================================================================

/// Value remaining inventory using LIFO.
///
/// Under LIFO, the newest layers are consumed first, so remaining inventory
/// carries the cost of the oldest purchases. We take total consumed quantity
/// and conceptually "remove" it from the newest layers, then value what remains
/// in the oldest layers.
///
/// Layers must be for a single item, sorted oldest-first.
pub fn value_lifo(layers: &[FullLayer]) -> Option<ItemValuation> {
    if layers.is_empty() {
        return None;
    }

    let item_id = layers[0].item_id;
    let total_received: i64 = layers.iter().map(|l| l.quantity_received).sum();
    let total_consumed: i64 = layers.iter().map(|l| l.qty_consumed_at_as_of).sum();
    let total_on_hand = total_received - total_consumed;

    if total_on_hand <= 0 {
        return None;
    }

    // LIFO: consume from newest first. Remaining = oldest layers.
    // Walk newest-to-oldest, subtracting consumed qty.
    let mut lifo_remaining_to_consume = total_consumed;
    // Track how much of each layer survives LIFO consumption
    let mut lifo_remaining: Vec<i64> = layers.iter().map(|l| l.quantity_received).collect();

    // Consume from newest (end of vec) to oldest (start)
    for i in (0..lifo_remaining.len()).rev() {
        if lifo_remaining_to_consume <= 0 {
            break;
        }
        let take = lifo_remaining_to_consume.min(lifo_remaining[i]);
        lifo_remaining[i] -= take;
        lifo_remaining_to_consume -= take;
    }

    let mut total_cost: i64 = 0;
    let mut total_qty: i64 = 0;
    for (i, layer) in layers.iter().enumerate() {
        let qty = lifo_remaining[i];
        if qty > 0 {
            total_qty += qty;
            total_cost += qty * layer.unit_cost_minor;
        }
    }

    if total_qty == 0 {
        return None;
    }

    let unit_cost = total_cost / total_qty;

    Some(ItemValuation {
        item_id,
        quantity_on_hand: total_qty,
        unit_cost_minor: unit_cost,
        total_value_minor: total_cost,
        variance_minor: 0,
    })
}

// ============================================================================
// WAC (Weighted Average Cost) valuation
// ============================================================================

/// Value remaining inventory using Weighted Average Cost.
///
/// WAC per unit = total cost of all receipts / total quantity received.
/// Ending inventory value = qty_on_hand * WAC.
///
/// Layers must be for a single item, sorted oldest-first.
pub fn value_wac(layers: &[FullLayer]) -> Option<ItemValuation> {
    if layers.is_empty() {
        return None;
    }

    let item_id = layers[0].item_id;
    let total_received: i64 = layers.iter().map(|l| l.quantity_received).sum();
    let total_consumed: i64 = layers.iter().map(|l| l.qty_consumed_at_as_of).sum();
    let total_on_hand = total_received - total_consumed;

    if total_on_hand <= 0 || total_received <= 0 {
        return None;
    }

    let total_receipt_cost: i64 = layers
        .iter()
        .map(|l| l.quantity_received * l.unit_cost_minor)
        .sum();

    let wac = total_receipt_cost / total_received;
    let total_value = total_on_hand * wac;

    Some(ItemValuation {
        item_id,
        quantity_on_hand: total_on_hand,
        unit_cost_minor: wac,
        total_value_minor: total_value,
        variance_minor: 0,
    })
}

// ============================================================================
// Standard Cost valuation
// ============================================================================

/// Value remaining inventory using Standard Cost.
///
/// Ending value = qty_on_hand * standard_cost_minor.
/// Variance = FIFO actual value - standard value.
///
/// Layers must be for a single item, sorted oldest-first.
pub fn value_standard_cost(
    layers: &[FullLayer],
    standard_cost_minor: i64,
) -> Option<ItemValuation> {
    if layers.is_empty() {
        return None;
    }

    let item_id = layers[0].item_id;
    let total_received: i64 = layers.iter().map(|l| l.quantity_received).sum();
    let total_consumed: i64 = layers.iter().map(|l| l.qty_consumed_at_as_of).sum();
    let total_on_hand = total_received - total_consumed;

    if total_on_hand <= 0 {
        return None;
    }

    let standard_value = total_on_hand * standard_cost_minor;

    // Compute actual FIFO value for variance calculation
    let mut actual_value: i64 = 0;
    for layer in layers {
        let remaining = layer.qty_remaining();
        if remaining > 0 {
            actual_value += remaining * layer.unit_cost_minor;
        }
    }

    let variance = actual_value - standard_value;

    Some(ItemValuation {
        item_id,
        quantity_on_hand: total_on_hand,
        unit_cost_minor: standard_cost_minor,
        total_value_minor: standard_value,
        variance_minor: variance,
    })
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_layer(item_id: Uuid, received: i64, consumed: i64, cost: i64) -> FullLayer {
        FullLayer {
            item_id,
            unit_cost_minor: cost,
            quantity_received: received,
            qty_consumed_at_as_of: consumed,
        }
    }

    fn item() -> Uuid {
        Uuid::new_v4()
    }

    // ── FIFO tests ──────────────────────────────────────────────────────

    #[test]
    fn fifo_single_layer_no_consumption() {
        let id = item();
        let layers = vec![make_layer(id, 10, 0, 500)];
        let v = value_fifo(&layers).expect("fifo single layer");
        assert_eq!(v.quantity_on_hand, 10);
        assert_eq!(v.unit_cost_minor, 500);
        assert_eq!(v.total_value_minor, 5000);
    }

    #[test]
    fn fifo_partial_consumption() {
        let id = item();
        let layers = vec![
            make_layer(id, 10, 10, 500), // fully consumed
            make_layer(id, 20, 5, 800),  // 15 remaining
        ];
        let v = value_fifo(&layers).expect("fifo partial");
        assert_eq!(v.quantity_on_hand, 15);
        assert_eq!(v.total_value_minor, 15 * 800);
    }

    #[test]
    fn fifo_empty_returns_none() {
        assert!(value_fifo(&[]).is_none());
    }

    #[test]
    fn fifo_all_consumed_returns_none() {
        let id = item();
        let layers = vec![make_layer(id, 10, 10, 500)];
        assert!(value_fifo(&layers).is_none());
    }

    // ── LIFO tests ──────────────────────────────────────────────────────

    #[test]
    fn lifo_consumes_newest_first() {
        let id = item();
        // Receipt 1 (oldest): 10 @ $5
        // Receipt 2 (newest): 20 @ $8
        // Total consumed: 15 (under LIFO, all from newest layer)
        let layers = vec![
            make_layer(id, 10, 0, 500),
            make_layer(id, 20, 15, 800), // physical FIFO consumed 15 here
        ];
        // LIFO: consume 15 from newest → 20-15=5 remain in newest,
        // all 10 remain in oldest
        // But total on_hand = 10+20 - 15 = 15
        // LIFO remaining: 10@$5 + 5@$8 = 5000 + 4000 = 9000
        let v = value_lifo(&layers).expect("lifo consumes newest");
        assert_eq!(v.quantity_on_hand, 15);
        assert_eq!(v.total_value_minor, 9000);
    }

    #[test]
    fn lifo_all_consumed_from_newest() {
        let id = item();
        // 3 layers, 25 consumed
        // Layer 1: 10 @ $5, Layer 2: 20 @ $8, Layer 3: 15 @ $10
        // Total: 45, consumed: 25, on_hand: 20
        let layers = vec![
            make_layer(id, 10, 0, 500),
            make_layer(id, 20, 10, 800),
            make_layer(id, 15, 15, 1000),
        ];
        // LIFO: consume 25 from newest-first:
        //   layer 3: consume 15, remaining 0
        //   layer 2: consume 10, remaining 10
        //   layer 1: remaining 10
        // Value: 10*500 + 10*800 = 5000 + 8000 = 13000
        let v = value_lifo(&layers).expect("lifo all consumed newest");
        assert_eq!(v.quantity_on_hand, 20);
        assert_eq!(v.total_value_minor, 13000);
    }

    #[test]
    fn lifo_empty_returns_none() {
        assert!(value_lifo(&[]).is_none());
    }

    // ── WAC tests ───────────────────────────────────────────────────────

    #[test]
    fn wac_single_cost() {
        let id = item();
        let layers = vec![make_layer(id, 10, 0, 500)];
        let v = value_wac(&layers).expect("wac single cost");
        assert_eq!(v.unit_cost_minor, 500);
        assert_eq!(v.total_value_minor, 5000);
    }

    #[test]
    fn wac_mixed_costs() {
        let id = item();
        // 10 @ $5 + 20 @ $8 = $50 + $160 = $210
        // WAC = $210 / 30 = $7 per unit
        // On hand: 30 - 5 = 25
        // Value: 25 * $7 = $175
        let layers = vec![make_layer(id, 10, 0, 500), make_layer(id, 20, 5, 800)];
        let v = value_wac(&layers).expect("wac mixed costs");
        assert_eq!(v.quantity_on_hand, 25);
        // WAC: (10*500 + 20*800) / (10+20) = 21000/30 = 700
        assert_eq!(v.unit_cost_minor, 700);
        assert_eq!(v.total_value_minor, 25 * 700);
    }

    #[test]
    fn wac_empty_returns_none() {
        assert!(value_wac(&[]).is_none());
    }

    // ── Standard Cost tests ─────────────────────────────────────────────

    #[test]
    fn standard_cost_basic() {
        let id = item();
        let layers = vec![make_layer(id, 10, 0, 500)];
        let v = value_standard_cost(&layers, 600).expect("standard cost basic");
        assert_eq!(v.quantity_on_hand, 10);
        assert_eq!(v.unit_cost_minor, 600);
        assert_eq!(v.total_value_minor, 6000);
        // Variance: actual (10*500=5000) - standard (6000) = -1000
        assert_eq!(v.variance_minor, -1000);
    }

    #[test]
    fn standard_cost_positive_variance() {
        let id = item();
        let layers = vec![make_layer(id, 10, 0, 800)];
        let v = value_standard_cost(&layers, 600).expect("standard cost variance");
        // Actual: 10*800=8000, Standard: 10*600=6000
        // Variance: 8000 - 6000 = 2000 (unfavorable)
        assert_eq!(v.variance_minor, 2000);
    }

    #[test]
    fn standard_cost_empty_returns_none() {
        assert!(value_standard_cost(&[], 500).is_none());
    }

    // ── Method comparison ───────────────────────────────────────────────

    #[test]
    fn methods_produce_different_values() {
        let id = item();
        // Layer 1 (oldest): 10 @ $5 = $50
        // Layer 2 (mid):    20 @ $8 = $160
        // Layer 3 (newest): 15 @ $10 = $150
        // Total received: 45, consumed: 25, on hand: 20
        let layers = vec![
            make_layer(id, 10, 10, 500), // FIFO: fully consumed
            make_layer(id, 20, 10, 800), // FIFO: 10 remaining
            make_layer(id, 15, 5, 1000), // FIFO: 10 remaining
        ];

        let fifo = value_fifo(&layers).expect("fifo comparison");
        let lifo = value_lifo(&layers).expect("lifo comparison");
        let wac = value_wac(&layers).expect("wac comparison");
        let std_cost = value_standard_cost(&layers, 700).expect("standard comparison");

        // All methods agree on quantity
        assert_eq!(fifo.quantity_on_hand, 20);
        assert_eq!(lifo.quantity_on_hand, 20);
        assert_eq!(wac.quantity_on_hand, 20);
        assert_eq!(std_cost.quantity_on_hand, 20);

        // But disagree on total value
        // FIFO: 10*800 + 10*1000 = 18000
        assert_eq!(fifo.total_value_minor, 18000);
        // LIFO: 10*500 + 10*800 = 13000
        assert_eq!(lifo.total_value_minor, 13000);
        // WAC: (10*500 + 20*800 + 15*1000) / 45 = 360/unit → 20*800 = 16000
        // Actually: 5000+16000+15000 = 36000 / 45 = 800
        // 20 * 800 = 16000
        assert_eq!(wac.total_value_minor, 16000);
        // Standard: 20 * 700 = 14000
        assert_eq!(std_cost.total_value_minor, 14000);

        // Verify they're all different
        let values = [
            fifo.total_value_minor,
            lifo.total_value_minor,
            wac.total_value_minor,
            std_cost.total_value_minor,
        ];
        for i in 0..values.len() {
            for j in (i + 1)..values.len() {
                assert_ne!(
                    values[i], values[j],
                    "methods {} and {} should differ",
                    i, j
                );
            }
        }
    }

    // ── ValuationMethod enum ────────────────────────────────────────────

    #[test]
    fn method_roundtrip() {
        assert_eq!(ValuationMethod::try_from("fifo"), Ok(ValuationMethod::Fifo));
        assert_eq!(ValuationMethod::try_from("lifo"), Ok(ValuationMethod::Lifo));
        assert_eq!(ValuationMethod::try_from("wac"), Ok(ValuationMethod::Wac));
        assert_eq!(
            ValuationMethod::try_from("standard_cost"),
            Ok(ValuationMethod::StandardCost)
        );
        assert!(ValuationMethod::try_from("invalid").is_err());
    }

    #[test]
    fn method_display() {
        assert_eq!(ValuationMethod::Fifo.as_str(), "fifo");
        assert_eq!(ValuationMethod::Lifo.as_str(), "lifo");
        assert_eq!(ValuationMethod::Wac.as_str(), "wac");
        assert_eq!(ValuationMethod::StandardCost.as_str(), "standard_cost");
    }
}
