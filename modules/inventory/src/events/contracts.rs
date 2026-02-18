//! Inventory event type constants and payload structs
//!
//! Defines the canonical event contracts for inventory events:
//! - inventory.item_received       (stock received into a warehouse)
//! - inventory.item_issued         (stock issued/consumed; GL consumes for COGS)
//! - inventory.adjusted            (manual stock adjustment, positive or negative)
//! - inventory.transfer_completed  (inter-warehouse transfer completed)
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0"
//! - mutation_class: per event (DATA_MUTATION)
//! - correlation_id / causation_id: caller-supplied for tracing
//! - event_id: caller-supplied for idempotency (deterministic from business key)
//! - replay_safe: true (all inventory events are idempotent by event_id)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constants
// ============================================================================

/// Stock was received into a warehouse (purchase receipt, inbound shipment)
pub const EVENT_TYPE_ITEM_RECEIVED: &str = "inventory.item_received";

/// Stock was issued/consumed (sale fulfillment, internal consumption)
/// GL module consumes this event directly for COGS booking.
pub const EVENT_TYPE_ITEM_ISSUED: &str = "inventory.item_issued";

/// Manual stock adjustment applied (positive or negative delta)
pub const EVENT_TYPE_ADJUSTED: &str = "inventory.adjusted";

/// Inter-warehouse transfer has completed (both debit and credit legs posted)
pub const EVENT_TYPE_TRANSFER_COMPLETED: &str = "inventory.transfer_completed";

// ============================================================================
// Payload: inventory.item_received
// ============================================================================

/// Payload for inventory.item_received
///
/// Emitted when physical stock is received into a warehouse.
/// Idempotency: caller MUST supply a deterministic event_id from the receipt line key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemReceivedPayload {
    /// Stable business key for this receipt line (idempotency anchor)
    pub receipt_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    /// Stock-keeping unit code
    pub sku: String,
    pub warehouse_id: Uuid,
    /// Quantity received (always positive)
    pub quantity: i64,
    /// Unit cost in minor currency units (e.g. cents)
    pub unit_cost_minor: i64,
    pub currency: String,
    /// Source purchase order, if applicable
    pub purchase_order_id: Option<Uuid>,
    pub received_at: DateTime<Utc>,
}

/// Build an envelope for inventory.item_received
pub fn build_item_received_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ItemReceivedPayload,
) -> EventEnvelope<ItemReceivedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ITEM_RECEIVED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INVENTORY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: inventory.item_issued — supporting types (locked schema)
// ============================================================================

/// One FIFO layer consumed during an issue.
///
/// `extended_cost_minor` is precomputed: `quantity * unit_cost_minor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumedLayer {
    pub layer_id: Uuid,
    /// Units drawn from this layer (always > 0)
    pub quantity: i64,
    /// Cost per unit from this layer, minor currency units
    pub unit_cost_minor: i64,
    /// Precomputed: quantity × unit_cost_minor
    pub extended_cost_minor: i64,
}

/// Source reference: who triggered this issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub source_module: String,
    pub source_type: String,
    pub source_id: String,
    pub source_line_id: Option<String>,
}

// ============================================================================
// Payload: inventory.item_issued
// ============================================================================

/// Payload for inventory.item_issued
///
/// Emitted when stock is issued (consumed, shipped to a customer, etc.).
/// GL module subscribes to this event to post the COGS journal entry.
///
/// Locked schema:
/// - `consumed_layers` MUST be non-empty.
/// - `source_ref` MUST be present.
/// - `total_cost_minor` MUST equal sum of consumed_layers.extended_cost_minor.
///
/// Idempotency: caller MUST supply a deterministic event_id from the issue line key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemIssuedPayload {
    /// Stable business key for this issue line (idempotency anchor)
    pub issue_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    /// Total quantity issued (always positive)
    pub quantity: i64,
    /// Total COGS value = sum of consumed_layers.extended_cost_minor
    pub total_cost_minor: i64,
    pub currency: String,
    /// FIFO layer breakdown — REQUIRED, non-empty
    pub consumed_layers: Vec<ConsumedLayer>,
    /// Source document reference — REQUIRED
    pub source_ref: SourceRef,
    pub issued_at: DateTime<Utc>,
}

/// Build an envelope for inventory.item_issued
pub fn build_item_issued_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ItemIssuedPayload,
) -> EventEnvelope<ItemIssuedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ITEM_ISSUED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INVENTORY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: inventory.adjusted
// ============================================================================

/// Payload for inventory.adjusted
///
/// Emitted when a manual stock adjustment is applied to correct on-hand quantity.
/// Idempotency: caller MUST supply a deterministic event_id from the adjustment key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjustedPayload {
    /// Stable business key for this adjustment (idempotency anchor)
    pub adjustment_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    /// Change in quantity (positive = gain, negative = shrinkage/write-off)
    pub quantity_delta: i64,
    /// Human-readable reason for the adjustment
    pub reason: String,
    pub adjusted_at: DateTime<Utc>,
}

/// Build an envelope for inventory.adjusted
pub fn build_adjusted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: AdjustedPayload,
) -> EventEnvelope<AdjustedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ADJUSTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INVENTORY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: inventory.transfer_completed
// ============================================================================

/// Payload for inventory.transfer_completed
///
/// Emitted when both legs of an inter-warehouse transfer have been posted
/// (debit from source, credit to destination).
/// Idempotency: caller MUST supply a deterministic event_id from the transfer key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferCompletedPayload {
    /// Stable business key for this transfer (idempotency anchor)
    pub transfer_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    /// Source warehouse (stock was debited from here)
    pub from_warehouse_id: Uuid,
    /// Destination warehouse (stock was credited here)
    pub to_warehouse_id: Uuid,
    /// Quantity transferred (always positive)
    pub quantity: i64,
    pub transferred_at: DateTime<Utc>,
}

/// Build an envelope for inventory.transfer_completed
pub fn build_transfer_completed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: TransferCompletedPayload,
) -> EventEnvelope<TransferCompletedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_TRANSFER_COMPLETED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INVENTORY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ---- inventory.item_received ----

    #[test]
    fn item_received_envelope_has_correct_metadata() {
        let payload = ItemReceivedPayload {
            receipt_line_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-001".to_string(),
            warehouse_id: Uuid::new_v4(),
            quantity: 100,
            unit_cost_minor: 5000,
            currency: "usd".to_string(),
            purchase_order_id: None,
            received_at: Utc::now(),
        };
        let envelope = build_item_received_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ITEM_RECEIVED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }

    // ---- inventory.item_issued ----

    fn make_consumed_layer(qty: i64, cost: i64) -> ConsumedLayer {
        ConsumedLayer {
            layer_id: Uuid::new_v4(),
            quantity: qty,
            unit_cost_minor: cost,
            extended_cost_minor: qty * cost,
        }
    }

    fn make_source_ref() -> SourceRef {
        SourceRef {
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-001".to_string(),
            source_line_id: Some("SO-001-L1".to_string()),
        }
    }

    #[test]
    fn item_issued_envelope_has_correct_metadata() {
        let layer = make_consumed_layer(10, 5000);
        let total_cost = layer.extended_cost_minor;
        let payload = ItemIssuedPayload {
            issue_line_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-001".to_string(),
            warehouse_id: Uuid::new_v4(),
            quantity: 10,
            total_cost_minor: total_cost,
            currency: "usd".to_string(),
            consumed_layers: vec![layer],
            source_ref: make_source_ref(),
            issued_at: Utc::now(),
        };
        let envelope = build_item_issued_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ITEM_ISSUED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "inventory");
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
        assert!(envelope.replay_safe);
    }

    // ---- inventory.adjusted ----

    #[test]
    fn adjusted_envelope_has_correct_metadata() {
        let payload = AdjustedPayload {
            adjustment_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-002".to_string(),
            warehouse_id: Uuid::new_v4(),
            quantity_delta: -5,
            reason: "shrinkage".to_string(),
            adjusted_at: Utc::now(),
        };
        let envelope = build_adjusted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-3".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ADJUSTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }

    // ---- inventory.transfer_completed ----

    #[test]
    fn transfer_completed_envelope_has_correct_metadata() {
        let payload = TransferCompletedPayload {
            transfer_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-003".to_string(),
            from_warehouse_id: Uuid::new_v4(),
            to_warehouse_id: Uuid::new_v4(),
            quantity: 50,
            transferred_at: Utc::now(),
        };
        let envelope = build_transfer_completed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-4".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_TRANSFER_COMPLETED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }

    // ---- no cogs_post_required event ----

    #[test]
    fn no_cogs_post_required_event_type_defined() {
        // Verify cogs_post_required is not a defined event constant
        // GL consumes inventory.item_issued directly
        let event_types = [
            EVENT_TYPE_ITEM_RECEIVED,
            EVENT_TYPE_ITEM_ISSUED,
            EVENT_TYPE_ADJUSTED,
            EVENT_TYPE_TRANSFER_COMPLETED,
        ];
        for et in &event_types {
            assert!(!et.contains("cogs_post_required"), "unexpected cogs_post_required event: {}", et);
        }
    }

    // ---- payload serialization ----

    #[test]
    fn item_issued_payload_serializes_correctly() {
        let layer = make_consumed_layer(3, 2000);
        let total_cost = layer.extended_cost_minor;
        let payload = ItemIssuedPayload {
            issue_line_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-001".to_string(),
            warehouse_id: Uuid::new_v4(),
            quantity: 3,
            total_cost_minor: total_cost,
            currency: "usd".to_string(),
            consumed_layers: vec![layer],
            source_ref: make_source_ref(),
            issued_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("issue_line_id"));
        assert!(json.contains("SKU-001"));
        assert!(json.contains("total_cost_minor"));
        assert!(json.contains("consumed_layers"));
    }
}
