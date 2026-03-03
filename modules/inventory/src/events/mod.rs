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

pub mod classification_assigned;
pub mod contracts;
pub mod cycle_count_approved;
pub mod cycle_count_submitted;
pub mod expiry_alert;
pub mod expiry_set;
pub mod item_change_recorded;
pub mod label_generated;
pub mod lot_merged;
pub mod lot_split;
pub mod low_stock_triggered;
pub mod revision_activated;
pub mod revision_created;
pub mod revision_policy_updated;
pub mod status_changed;
pub mod valuation_run_completed;
pub mod valuation_snapshot_created;

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
    build_adjusted_envelope, build_item_issued_envelope, build_item_received_envelope,
    build_transfer_completed_envelope, AdjustedPayload, ConsumedLayer, ItemIssuedPayload,
    ItemReceivedPayload, SourceRef, TransferCompletedPayload, EVENT_TYPE_ADJUSTED,
    EVENT_TYPE_ITEM_ISSUED, EVENT_TYPE_ITEM_RECEIVED, EVENT_TYPE_TRANSFER_COMPLETED,
};

pub use cycle_count_approved::{
    build_cycle_count_approved_envelope, CycleCountApprovedLine, CycleCountApprovedPayload,
    EVENT_TYPE_CYCLE_COUNT_APPROVED,
};

pub use cycle_count_submitted::{
    build_cycle_count_submitted_envelope, CycleCountSubmittedLine, CycleCountSubmittedPayload,
    EVENT_TYPE_CYCLE_COUNT_SUBMITTED,
};

pub use expiry_alert::{build_expiry_alert_envelope, ExpiryAlertPayload, EVENT_TYPE_EXPIRY_ALERT};

pub use expiry_set::{build_expiry_set_envelope, ExpirySetPayload, EVENT_TYPE_EXPIRY_SET};

pub use low_stock_triggered::{
    build_low_stock_triggered_envelope, LowStockTriggeredPayload, EVENT_TYPE_LOW_STOCK_TRIGGERED,
};

pub use status_changed::{
    build_status_changed_envelope, StatusChangedPayload, EVENT_TYPE_STATUS_CHANGED,
};

pub use valuation_snapshot_created::{
    build_valuation_snapshot_created_envelope, ValuationSnapshotCreatedLine,
    ValuationSnapshotCreatedPayload, EVENT_TYPE_VALUATION_SNAPSHOT_CREATED,
};

pub use revision_created::{
    build_item_revision_created_envelope, ItemRevisionCreatedPayload,
    EVENT_TYPE_ITEM_REVISION_CREATED,
};

pub use revision_activated::{
    build_item_revision_activated_envelope, ItemRevisionActivatedPayload,
    EVENT_TYPE_ITEM_REVISION_ACTIVATED,
};

pub use revision_policy_updated::{
    build_item_revision_policy_updated_envelope, ItemRevisionPolicyUpdatedPayload,
    EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED,
};

pub use label_generated::{
    build_label_generated_envelope, LabelGeneratedPayload, EVENT_TYPE_LABEL_GENERATED,
};

pub use lot_split::{
    build_lot_split_envelope, LotSplitPayload, SplitChildEdge, EVENT_TYPE_LOT_SPLIT,
};

pub use lot_merged::{
    build_lot_merged_envelope, LotMergedPayload, MergeParentEdge, EVENT_TYPE_LOT_MERGED,
};

pub use item_change_recorded::{
    build_item_change_recorded_envelope, ItemChangeRecordedPayload,
    EVENT_TYPE_ITEM_CHANGE_RECORDED,
};

pub use valuation_run_completed::{
    build_valuation_run_completed_envelope, ValuationRunCompletedLine,
    ValuationRunCompletedPayload, EVENT_TYPE_VALUATION_RUN_COMPLETED,
};

pub use classification_assigned::{
    build_classification_assigned_envelope, ClassificationAssignedPayload,
    EVENT_TYPE_CLASSIFICATION_ASSIGNED,
};

// ============================================================================
// Envelope builder helper
// ============================================================================

/// Create an inventory-scoped EventEnvelope.
///
/// Sets `source_module = "inventory"` and `replay_safe = true`.
/// Callers MUST supply a deterministic `event_id` derived from a stable business key.
/// **Phase 34**: trace_id auto-populated from correlation_id for propagation
/// **Phase 40**: actor_id/actor_type carried from VerifiedClaims on HTTP mutations
pub fn create_inventory_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> event_bus::EventEnvelope<T> {
    create_inventory_envelope_with_actor(
        event_id,
        tenant_id,
        event_type,
        correlation_id,
        causation_id,
        mutation_class,
        payload,
        None,
        None,
    )
}

/// Create an inventory-scoped EventEnvelope with actor identity.
///
/// Actor fields are propagated from the originating HTTP request's VerifiedClaims.
/// Pass `None` for both fields when the operation is system-initiated.
pub fn create_inventory_envelope_with_actor<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
    actor_id: Option<uuid::Uuid>,
    actor_type: Option<String>,
) -> event_bus::EventEnvelope<T> {
    event_bus::EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "inventory".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(correlation_id.clone()))
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(mutation_class))
    .with_replay_safe(true)
    .with_actor_from(actor_id, actor_type)
}
