//! Stock adjustment service.
//!
//! Corrects physical reality without editing history.
//! Each adjustment creates:
//!   - A new inventory_ledger row (entry_type = 'adjusted', signed quantity)
//!   - A new inv_adjustments row (business key + reason)
//!   - An item_on_hand projection update (quantity_on_hand += delta)
//!   - An item_on_hand_by_status update (available bucket += delta)
//!   - An inventory.adjusted outbox event
//!
//! Guards:
//!   - Item must be active
//!   - quantity_delta != 0
//!   - reason must be non-empty
//!   - No-negative policy: negative delta requires on_hand >= abs(delta)
//!     unless allow_negative = true
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)

mod service;
mod types;

// Re-export public API to preserve existing import paths.
pub use service::process_adjustment;
pub use types::{AdjustError, AdjustRequest, AdjustResult};
