//! Atomic stock issue service.
//!
//! Invariants:
//! - Lock per (tenant_id, item_id, warehouse_id) via SELECT … FOR UPDATE on FIFO layers.
//! - Available = sum(layer.quantity_remaining) − quantity_reserved (no negatives allowed).
//! - Deterministic FIFO consumption: oldest layer first, tie-break by ledger_entry_id.
//! - Ledger row + layer_consumptions + layer updates + on-hand projection + outbox event
//!   created in a single transaction.
//! - Idempotency key prevents double-processing on retry.
//!
//! ## Lot/Serial Tracking Policy
//! - Lot-tracked items MUST supply `lot_code`. FIFO is restricted to layers in that lot.
//! - Serial-tracked items MUST supply `serial_codes`. Quantity is derived from the list.
//! - None-tracked items use warehouse-wide FIFO (existing behavior).
//!
//! Pattern: Guard → Lock → FIFO → Mutation → Outbox (all in one transaction).

mod idempotency;
pub(crate) mod service;
mod types;

// Re-export public API to preserve existing import paths.
pub use service::process_issue;
pub use types::{IssueError, IssueRequest, IssueResult};
