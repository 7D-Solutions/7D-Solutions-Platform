//! Inventory event contracts (schema_version = 1)
//!
//! Provides canonical event_type strings, payload structs, and envelope builder
//! helpers for all events emitted by the inventory module.
//!
//! ## Event Types
//!
//! | Event Type                        | mutation_class | Consumer    |
//! |-----------------------------------|----------------|-------------|
//! | inventory.item_received           | DATA_MUTATION  | projections |
//! | inventory.item_issued             | DATA_MUTATION  | GL (COGS)   |
//! | inventory.adjusted                | DATA_MUTATION  | projections |
//! | inventory.transfer_completed      | DATA_MUTATION  | projections |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use inventory_rs::events::contracts::{
//!     ConsumedLayer, ItemIssuedPayload, SourceRef, build_item_issued_envelope,
//! };
//! use uuid::Uuid;
//! use chrono::Utc;
//!
//! let layer = ConsumedLayer { layer_id: Uuid::new_v4(), quantity: 5, unit_cost_minor: 2000, extended_cost_minor: 10000 };
//! let payload = ItemIssuedPayload {
//!     issue_line_id: Uuid::new_v4(),
//!     tenant_id: "tenant-1".to_string(),
//!     item_id: Uuid::new_v4(),
//!     sku: "SKU-001".to_string(),
//!     warehouse_id: Uuid::new_v4(),
//!     quantity: 5,
//!     total_cost_minor: 10000,
//!     currency: "usd".to_string(),
//!     consumed_layers: vec![layer],
//!     source_ref: SourceRef { source_module: "orders".to_string(), source_type: "sales_order".to_string(), source_id: "SO-1".to_string(), source_line_id: None },
//!     issued_at: Utc::now(),
//! };
//! let envelope = build_item_issued_envelope(
//!     Uuid::new_v4(),
//!     "tenant-1".to_string(),
//!     "trace-abc".to_string(),
//!     None,
//!     payload,
//! );
//! assert_eq!(envelope.source_module, "inventory");
//! ```

pub mod contracts;
pub mod status_changed;

// ============================================================================
// Shared Constants
// ============================================================================

/// Schema version for all inventory event payloads (v1)
pub const INVENTORY_EVENT_SCHEMA_VERSION: &str = "1.0.0";

/// DATA_MUTATION: creates or modifies an inventory record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

// ============================================================================
// Re-exports
// ============================================================================

pub use contracts::{
    AdjustedPayload, ConsumedLayer, ItemIssuedPayload, ItemReceivedPayload,
    SourceRef, TransferCompletedPayload,
    EVENT_TYPE_ADJUSTED, EVENT_TYPE_ITEM_ISSUED, EVENT_TYPE_ITEM_RECEIVED,
    EVENT_TYPE_TRANSFER_COMPLETED,
    build_adjusted_envelope, build_item_issued_envelope, build_item_received_envelope,
    build_transfer_completed_envelope,
};

pub use status_changed::{
    StatusChangedPayload, EVENT_TYPE_STATUS_CHANGED, build_status_changed_envelope,
};

// ============================================================================
// Envelope builder helper
// ============================================================================

/// Create an inventory-scoped EventEnvelope.
///
/// Sets `source_module = "inventory"` and `replay_safe = true`.
/// Callers MUST supply a deterministic `event_id` derived from a stable business key.
pub fn create_inventory_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> event_bus::EventEnvelope<T> {
    event_bus::EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "inventory".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(mutation_class))
    .with_replay_safe(true)
}
