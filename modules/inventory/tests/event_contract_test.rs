/// Comprehensive event contract tests for all inventory event types.
///
/// Validates that every event type published by the inventory module
/// produces a conformant EventEnvelope with:
///   - correct event_type constant
///   - source_module = "inventory"
///   - schema_version = "1.0.0"
///   - mutation_class = Some("DATA_MUTATION")
///   - replay_safe = true
///   - correlation_id propagated
///   - causation_id propagated when supplied
///   - deterministic event_id preserved
///   - tenant_id preserved
///   - payload JSON round-trip succeeds
use chrono::Utc;
use uuid::Uuid;

use inventory_rs::events::{
    build_adjusted_envelope, build_cycle_count_approved_envelope,
    build_cycle_count_submitted_envelope, build_item_issued_envelope,
    build_item_received_envelope, build_low_stock_triggered_envelope,
    build_status_changed_envelope, build_transfer_completed_envelope,
    build_valuation_snapshot_created_envelope, AdjustedPayload, ConsumedLayer,
    CycleCountApprovedLine, CycleCountApprovedPayload, CycleCountSubmittedLine,
    CycleCountSubmittedPayload, ItemIssuedPayload, ItemReceivedPayload,
    LowStockTriggeredPayload, SourceRef, StatusChangedPayload, TransferCompletedPayload,
    ValuationSnapshotCreatedLine, ValuationSnapshotCreatedPayload,
    EVENT_TYPE_ADJUSTED, EVENT_TYPE_CYCLE_COUNT_APPROVED, EVENT_TYPE_CYCLE_COUNT_SUBMITTED,
    EVENT_TYPE_ITEM_ISSUED, EVENT_TYPE_ITEM_RECEIVED, EVENT_TYPE_LOW_STOCK_TRIGGERED,
    EVENT_TYPE_STATUS_CHANGED, EVENT_TYPE_TRANSFER_COMPLETED,
    EVENT_TYPE_VALUATION_SNAPSHOT_CREATED, INVENTORY_EVENT_SCHEMA_VERSION,
    MUTATION_CLASS_DATA_MUTATION,
};

// ── Shared assertion helper ──────────────────────────────────────────

/// Assert the full EventEnvelope contract for any inventory event.
macro_rules! assert_envelope_contract {
    ($envelope:expr, $expected_event_type:expr, $event_id:expr, $tenant_id:expr) => {
        assert_eq!(
            $envelope.event_type, $expected_event_type,
            "event_type mismatch"
        );
        assert_eq!(
            $envelope.source_module, "inventory",
            "source_module must be 'inventory'"
        );
        assert_eq!(
            $envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION,
            "schema_version must be '{}'",
            INVENTORY_EVENT_SCHEMA_VERSION
        );
        assert_eq!(
            $envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION),
            "mutation_class must be Some(\"DATA_MUTATION\")"
        );
        assert!(
            $envelope.replay_safe,
            "replay_safe must be true for all inventory events"
        );
        assert_eq!(
            $envelope.event_id, $event_id,
            "event_id must be deterministic (passed-in value preserved)"
        );
        assert_eq!(
            $envelope.tenant_id, $tenant_id,
            "tenant_id must be preserved"
        );
        assert!(
            $envelope.correlation_id.is_some(),
            "correlation_id must be populated"
        );
        assert!(
            !$envelope.source_version.is_empty(),
            "source_version must be non-empty"
        );
    };
}

// ── 1. inventory.item_received ───────────────────────────────────────

#[test]
fn item_received_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-RECV-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 100,
        unit_cost_minor: 5000,
        currency: "usd".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        received_at: Utc::now(),
    };
    let env = build_item_received_envelope(
        event_id,
        tenant.to_string(),
        "corr-recv".to_string(),
        Some("cause-recv".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_ITEM_RECEIVED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-recv"));

    // Payload round-trip
    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: ItemReceivedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.sku, "SKU-RECV-001");
    assert_eq!(rt.quantity, 100);
    assert_eq!(rt.unit_cost_minor, 5000);
}

// ── 2. inventory.item_issued ─────────────────────────────────────────

#[test]
fn item_issued_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let layer = ConsumedLayer {
        layer_id: Uuid::new_v4(),
        quantity: 10,
        unit_cost_minor: 2000,
        extended_cost_minor: 20_000,
    };
    let payload = ItemIssuedPayload {
        issue_line_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-ISS-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        total_cost_minor: 20_000,
        currency: "usd".to_string(),
        consumed_layers: vec![layer],
        source_ref: SourceRef {
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-CT-001".to_string(),
            source_line_id: Some("SO-CT-001-L1".to_string()),
        },
        issued_at: Utc::now(),
    };
    let env = build_item_issued_envelope(
        event_id,
        tenant.to_string(),
        "corr-iss".to_string(),
        Some("cause-iss".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_ITEM_ISSUED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-iss"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: ItemIssuedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.sku, "SKU-ISS-001");
    assert_eq!(rt.consumed_layers.len(), 1);
    assert_eq!(rt.total_cost_minor, 20_000);
}

// ── 3. inventory.adjusted ────────────────────────────────────────────

#[test]
fn adjusted_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = AdjustedPayload {
        adjustment_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-ADJ-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity_delta: -5,
        reason: "shrinkage".to_string(),
        adjusted_at: Utc::now(),
    };
    let env = build_adjusted_envelope(
        event_id,
        tenant.to_string(),
        "corr-adj".to_string(),
        Some("cause-adj".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_ADJUSTED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-adj"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: AdjustedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.quantity_delta, -5);
    assert_eq!(rt.reason, "shrinkage");
}

// ── 4. inventory.transfer_completed ──────────────────────────────────

#[test]
fn transfer_completed_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = TransferCompletedPayload {
        transfer_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-XFR-001".to_string(),
        from_warehouse_id: Uuid::new_v4(),
        to_warehouse_id: Uuid::new_v4(),
        quantity: 50,
        transferred_at: Utc::now(),
    };
    let env = build_transfer_completed_envelope(
        event_id,
        tenant.to_string(),
        "corr-xfr".to_string(),
        Some("cause-xfr".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_TRANSFER_COMPLETED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-xfr"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: TransferCompletedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.quantity, 50);
}

// ── 5. inventory.low_stock_triggered ─────────────────────────────────

#[test]
fn low_stock_triggered_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = LowStockTriggeredPayload {
        tenant_id: tenant.to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        location_id: Some(Uuid::new_v4()),
        reorder_point: 50,
        available_qty: 30,
        triggered_at: Utc::now(),
    };
    let env = build_low_stock_triggered_envelope(
        event_id,
        tenant.to_string(),
        "corr-low".to_string(),
        Some("cause-low".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_LOW_STOCK_TRIGGERED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-low"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: LowStockTriggeredPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.reorder_point, 50);
    assert_eq!(rt.available_qty, 30);
    assert!(rt.location_id.is_some());
}

#[test]
fn low_stock_triggered_null_location_round_trips() {
    let payload = LowStockTriggeredPayload {
        tenant_id: "t1".to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        location_id: None,
        reorder_point: 10,
        available_qty: 5,
        triggered_at: Utc::now(),
    };
    let json = serde_json::to_value(&payload).expect("serialize");
    let rt: LowStockTriggeredPayload = serde_json::from_value(json).expect("deserialize");
    assert!(rt.location_id.is_none());
}

// ── 6. inventory.status_changed ──────────────────────────────────────

#[test]
fn status_changed_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = StatusChangedPayload {
        transfer_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-SC-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        from_status: "available".to_string(),
        to_status: "quarantine".to_string(),
        quantity: 10,
        transferred_at: Utc::now(),
    };
    let env = build_status_changed_envelope(
        event_id,
        tenant.to_string(),
        "corr-sc".to_string(),
        Some("cause-sc".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_STATUS_CHANGED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-sc"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: StatusChangedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.from_status, "available");
    assert_eq!(rt.to_status, "quarantine");
}

// ── 7. inventory.cycle_count_submitted ───────────────────────────────

#[test]
fn cycle_count_submitted_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = CycleCountSubmittedPayload {
        task_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        warehouse_id: Uuid::new_v4(),
        location_id: Uuid::new_v4(),
        submitted_at: Utc::now(),
        line_count: 2,
        lines: vec![
            CycleCountSubmittedLine {
                line_id: Uuid::new_v4(),
                item_id: Uuid::new_v4(),
                expected_qty: 100,
                counted_qty: 95,
                variance_qty: -5,
            },
            CycleCountSubmittedLine {
                line_id: Uuid::new_v4(),
                item_id: Uuid::new_v4(),
                expected_qty: 50,
                counted_qty: 52,
                variance_qty: 2,
            },
        ],
    };
    let env = build_cycle_count_submitted_envelope(
        event_id,
        tenant.to_string(),
        "corr-ccs".to_string(),
        Some("cause-ccs".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_CYCLE_COUNT_SUBMITTED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-ccs"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: CycleCountSubmittedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.line_count, 2);
    assert_eq!(rt.lines.len(), 2);
    assert_eq!(rt.lines[0].variance_qty, -5);
}

// ── 8. inventory.cycle_count_approved ────────────────────────────────

#[test]
fn cycle_count_approved_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = CycleCountApprovedPayload {
        task_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        warehouse_id: Uuid::new_v4(),
        location_id: Uuid::new_v4(),
        approved_at: Utc::now(),
        line_count: 1,
        adjustment_count: 1,
        lines: vec![CycleCountApprovedLine {
            line_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            expected_qty: 50,
            counted_qty: 45,
            variance_qty: -5,
            adjustment_id: Some(Uuid::new_v4()),
        }],
    };
    let env = build_cycle_count_approved_envelope(
        event_id,
        tenant.to_string(),
        "corr-cca".to_string(),
        Some("cause-cca".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_CYCLE_COUNT_APPROVED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-cca"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: CycleCountApprovedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.adjustment_count, 1);
    assert!(rt.lines[0].adjustment_id.is_some());
}

// ── 9. inventory.valuation_snapshot_created ───────────────────────────

#[test]
fn valuation_snapshot_created_full_contract() {
    let event_id = Uuid::new_v4();
    let tenant = "contract-test-tenant";
    let payload = ValuationSnapshotCreatedPayload {
        snapshot_id: Uuid::new_v4(),
        tenant_id: tenant.to_string(),
        warehouse_id: Uuid::new_v4(),
        location_id: None,
        as_of: Utc::now(),
        total_value_minor: 150_000,
        currency: "usd".to_string(),
        line_count: 2,
        lines: vec![
            ValuationSnapshotCreatedLine {
                item_id: Uuid::new_v4(),
                quantity_on_hand: 100,
                unit_cost_minor: 1000,
                total_value_minor: 100_000,
            },
            ValuationSnapshotCreatedLine {
                item_id: Uuid::new_v4(),
                quantity_on_hand: 50,
                unit_cost_minor: 1000,
                total_value_minor: 50_000,
            },
        ],
    };
    let env = build_valuation_snapshot_created_envelope(
        event_id,
        tenant.to_string(),
        "corr-vs".to_string(),
        Some("cause-vs".to_string()),
        payload,
    );
    assert_envelope_contract!(env, EVENT_TYPE_VALUATION_SNAPSHOT_CREATED, event_id, tenant);
    assert_eq!(env.causation_id.as_deref(), Some("cause-vs"));

    let json = serde_json::to_value(&env.payload).expect("serialize");
    let rt: ValuationSnapshotCreatedPayload = serde_json::from_value(json).expect("deserialize");
    assert_eq!(rt.line_count, 2);
    assert_eq!(rt.total_value_minor, 150_000);
    assert_eq!(rt.lines.len(), 2);
}

// ── Cross-cutting: causation_id=None leaves it unset ─────────────────

#[test]
fn causation_id_none_leaves_field_absent() {
    let env = build_item_received_envelope(
        Uuid::new_v4(),
        "t1".to_string(),
        "corr-none".to_string(),
        None,
        ItemReceivedPayload {
            receipt_line_id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-N".to_string(),
            warehouse_id: Uuid::new_v4(),
            quantity: 1,
            unit_cost_minor: 100,
            currency: "usd".to_string(),
            purchase_order_id: None,
            received_at: Utc::now(),
        },
    );
    assert!(env.causation_id.is_none());
}

// ── Cross-cutting: all event_type constants follow naming convention ──

#[test]
fn event_type_constants_follow_inventory_dot_convention() {
    let all_types = [
        EVENT_TYPE_ITEM_RECEIVED,
        EVENT_TYPE_ITEM_ISSUED,
        EVENT_TYPE_ADJUSTED,
        EVENT_TYPE_TRANSFER_COMPLETED,
        EVENT_TYPE_LOW_STOCK_TRIGGERED,
        EVENT_TYPE_STATUS_CHANGED,
        EVENT_TYPE_CYCLE_COUNT_SUBMITTED,
        EVENT_TYPE_CYCLE_COUNT_APPROVED,
        EVENT_TYPE_VALUATION_SNAPSHOT_CREATED,
    ];
    for et in &all_types {
        assert!(
            et.starts_with("inventory."),
            "event_type '{}' must start with 'inventory.'",
            et
        );
    }
    // Ensure no duplicates
    let mut sorted = all_types.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), all_types.len(), "duplicate event_type found");
}

// ── Cross-cutting: full envelope JSON round-trip ─────────────────────

#[test]
fn full_envelope_json_round_trip() {
    let event_id = Uuid::new_v4();
    let payload = ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: "rt-tenant".to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-RT".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 42,
        unit_cost_minor: 999,
        currency: "gbp".to_string(),
        purchase_order_id: None,
        received_at: Utc::now(),
    };
    let env = build_item_received_envelope(
        event_id,
        "rt-tenant".to_string(),
        "corr-rt".to_string(),
        Some("cause-rt".to_string()),
        payload,
    );

    // Serialize to JSON string
    let json_str = serde_json::to_string(&env).expect("envelope serialization");

    // Deserialize back
    let rt: event_bus::EventEnvelope<ItemReceivedPayload> =
        serde_json::from_str(&json_str).expect("envelope deserialization");

    assert_eq!(rt.event_id, event_id);
    assert_eq!(rt.event_type, EVENT_TYPE_ITEM_RECEIVED);
    assert_eq!(rt.source_module, "inventory");
    assert_eq!(rt.tenant_id, "rt-tenant");
    assert_eq!(rt.correlation_id.as_deref(), Some("corr-rt"));
    assert_eq!(rt.causation_id.as_deref(), Some("cause-rt"));
    assert_eq!(rt.payload.quantity, 42);
    assert_eq!(rt.payload.currency, "gbp");
    assert!(rt.replay_safe);
}
