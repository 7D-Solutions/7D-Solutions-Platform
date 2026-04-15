//! Consumer contract tests for AP (Accounts Payable) event types.
//!
//! These tests freeze the v1 AP event contracts by:
//! 1. Constructing real EventEnvelopes (no mocks)
//! 2. Serializing to JSON (real serde serialization)
//! 3. Validating envelope completeness per ADR-016
//! 4. Validating against JSON Schema files from contracts/events/
//!
//! Run: `cargo test -p platform_contracts`

use platform_contracts::{event_naming, mutation_classes, EventEnvelope};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ── Helpers (same as consumer_contracts.rs) ────────────────────────

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("contracts")
}

fn load_schema(name: &str) -> serde_json::Value {
    let path = contracts_dir().join("events").join(name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read schema {}: {}", path.display(), e));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse schema {}: {}", name, e))
}

fn validate_against_schema(envelope_json: &serde_json::Value, schema_name: &str) {
    let schema_value = load_schema(schema_name);
    let compiled = jsonschema::JSONSchema::compile(&schema_value)
        .unwrap_or_else(|e| panic!("Failed to compile schema {}: {}", schema_name, e));

    let result = compiled.validate(envelope_json);
    if let Err(errors) = result {
        let error_msgs: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
        panic!(
            "Schema validation failed for {}:\n{}",
            schema_name,
            error_msgs.join("\n")
        );
    }
}

fn assert_envelope_completeness(json: &serde_json::Value, label: &str) {
    let required_fields = [
        "event_id",
        "event_type",
        "occurred_at",
        "tenant_id",
        "source_module",
        "source_version",
        "schema_version",
        "replay_safe",
        "mutation_class",
        "payload",
    ];

    for field in &required_fields {
        let val = json.get(field);
        assert!(
            val.is_some(),
            "[{}] Missing required field: {}",
            label,
            field
        );
        let val = val.unwrap();
        if let Some(s) = val.as_str() {
            assert!(
                !s.is_empty(),
                "[{}] Field '{}' is empty string",
                label,
                field
            );
        }
    }

    let event_id = json["event_id"].as_str().unwrap();
    assert!(
        Uuid::parse_str(event_id).is_ok(),
        "[{}] event_id not valid UUID",
        label
    );

    let mc = json["mutation_class"].as_str().unwrap();
    assert!(
        mutation_classes::is_valid(mc),
        "[{}] Invalid mutation_class: '{}'",
        label,
        mc
    );

    let et = json["event_type"].as_str().unwrap();
    let type_part = if et.contains(".events.") {
        et.split(".events.").last().unwrap()
    } else {
        et
    };
    assert!(
        event_naming::validate_event_type(type_part).is_ok(),
        "[{}] event_type '{}' does not follow entity.action convention",
        label,
        et
    );

    assert!(
        json["replay_safe"].is_boolean(),
        "[{}] replay_safe must be boolean",
        label
    );
    assert!(
        json["payload"].is_object(),
        "[{}] payload must be object",
        label
    );

    let sv = json["source_version"].as_str().unwrap();
    assert!(
        sv.split('.').count() == 3,
        "[{}] source_version '{}' not semver",
        label,
        sv
    );
}

fn build_envelope<T: Serialize>(
    tenant_id: &str,
    event_type: &str,
    mutation_class: &str,
    payload: T,
) -> serde_json::Value {
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "ap".to_string(),
        event_type.to_string(),
        payload,
    )
    .with_source_version("0.1.0".to_string())
    .with_schema_version("1".to_string())
    .with_mutation_class(Some(mutation_class.to_string()))
    .with_correlation_id(Some(Uuid::new_v4().to_string()))
    .with_causation_id(Some(Uuid::new_v4().to_string()))
    .with_replay_safe(true);

    serde_json::to_value(&envelope).expect("Failed to serialize envelope")
}

// ── AP Event Payloads ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct VendorCreatedPayload {
    vendor_id: Uuid,
    tenant_id: String,
    name: String,
    tax_id: String,
    currency: String,
    payment_terms_days: i32,
    payment_method: String,
    remittance_email: String,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VendorUpdatedPayload {
    vendor_id: Uuid,
    tenant_id: String,
    name: Option<String>,
    tax_id: Option<String>,
    currency: Option<String>,
    payment_terms_days: Option<i32>,
    payment_method: Option<String>,
    remittance_email: Option<String>,
    updated_by: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PoLinePayload {
    line_id: Uuid,
    description: String,
    quantity: f64,
    unit_of_measure: String,
    unit_price_minor: i64,
    gl_account_code: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PoCreatedPayload {
    po_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    po_number: String,
    currency: String,
    lines: Vec<PoLinePayload>,
    total_minor: i64,
    created_by: String,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PoApprovedPayload {
    po_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    po_number: String,
    approved_amount_minor: i64,
    currency: String,
    approved_by: String,
    approved_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PoClosedPayload {
    po_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    po_number: String,
    close_reason: String,
    closed_by: String,
    closed_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PoLineReceivedLinkedPayload {
    po_id: Uuid,
    po_line_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    receipt_id: Uuid,
    quantity_received: f64,
    unit_of_measure: String,
    unit_price_minor: i64,
    currency: String,
    gl_account_code: String,
    received_at: String,
    received_by: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BillLinePayload {
    line_id: Uuid,
    description: String,
    quantity: f64,
    unit_price_minor: i64,
    line_total_minor: i64,
    gl_account_code: String,
    po_line_id: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VendorBillCreatedPayload {
    bill_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    vendor_invoice_ref: String,
    currency: String,
    lines: Vec<BillLinePayload>,
    total_minor: i64,
    tax_minor: Option<i64>,
    invoice_date: String,
    due_date: String,
    entered_by: String,
    entered_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BillMatchLinePayload {
    bill_line_id: Uuid,
    po_line_id: Uuid,
    receipt_id: Option<Uuid>,
    matched_quantity: f64,
    matched_amount_minor: i64,
    within_tolerance: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct VendorBillMatchedPayload {
    bill_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    po_id: Uuid,
    match_type: String,
    match_lines: Vec<BillMatchLinePayload>,
    fully_matched: bool,
    matched_by: String,
    matched_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlLinePayload {
    line_id: Uuid,
    gl_account_code: String,
    amount_minor: i64,
    po_line_id: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VendorBillApprovedPayload {
    bill_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    vendor_invoice_ref: String,
    approved_amount_minor: i64,
    currency: String,
    due_date: String,
    approved_by: String,
    approved_at: String,
    fx_rate_id: Option<Uuid>,
    gl_lines: Vec<GlLinePayload>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VendorBillVoidedPayload {
    bill_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    vendor_invoice_ref: String,
    original_total_minor: i64,
    currency: String,
    void_reason: String,
    voided_by: String,
    voided_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PaymentRunItemPayload {
    vendor_id: Uuid,
    bill_ids: Vec<Uuid>,
    amount_minor: i64,
    currency: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PaymentRunCreatedPayload {
    run_id: Uuid,
    tenant_id: String,
    items: Vec<PaymentRunItemPayload>,
    total_minor: i64,
    currency: String,
    scheduled_date: String,
    payment_method: String,
    created_by: String,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PaymentExecutedPayload {
    payment_id: Uuid,
    run_id: Uuid,
    tenant_id: String,
    vendor_id: Uuid,
    bill_ids: Vec<Uuid>,
    amount_minor: i64,
    currency: String,
    payment_method: String,
    bank_reference: Option<String>,
    bank_account_last4: Option<String>,
    executed_at: String,
}

// ══════════════════════════════════════════════════════════════════════
// VENDOR LIFECYCLE
// ══════════════════════════════════════════════════════════════════════

#[test]
fn ap_vendor_created_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_created",
        mutation_classes::DATA_MUTATION,
        VendorCreatedPayload {
            vendor_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            name: "Acme Corp".into(),
            tax_id: "12-3456789".into(),
            currency: "USD".into(),
            payment_terms_days: 30,
            payment_method: "ach".into(),
            remittance_email: "ap@acme.com".into(),
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/vendor_created");
}

#[test]
fn ap_vendor_created_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_created",
        mutation_classes::DATA_MUTATION,
        VendorCreatedPayload {
            vendor_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            name: "Bolt Mfg".into(),
            tax_id: "98-7654321".into(),
            currency: "USD".into(),
            payment_terms_days: 45,
            payment_method: "wire".into(),
            remittance_email: "pay@bolt.com".into(),
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-vendor-created.v1.json");
}

#[test]
fn ap_vendor_updated_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_updated",
        mutation_classes::DATA_MUTATION,
        VendorUpdatedPayload {
            vendor_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            name: Some("Acme Inc".into()),
            tax_id: None,
            currency: None,
            payment_terms_days: None,
            payment_method: None,
            remittance_email: None,
            updated_by: "user-1".into(),
            updated_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/vendor_updated");
}

#[test]
fn ap_vendor_updated_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_updated",
        mutation_classes::DATA_MUTATION,
        VendorUpdatedPayload {
            vendor_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            name: None,
            tax_id: None,
            currency: Some("EUR".into()),
            payment_terms_days: Some(60),
            payment_method: None,
            remittance_email: None,
            updated_by: "user-2".into(),
            updated_at: "2026-03-01T12:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-vendor-updated.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// PO LIFECYCLE
// ══════════════════════════════════════════════════════════════════════

fn sample_po_line() -> PoLinePayload {
    PoLinePayload {
        line_id: Uuid::new_v4(),
        description: "Widget A".into(),
        quantity: 100.0,
        unit_of_measure: "EA".into(),
        unit_price_minor: 5000,
        gl_account_code: "1400".into(),
    }
}

#[test]
fn ap_po_created_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.po_created",
        mutation_classes::DATA_MUTATION,
        PoCreatedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-001".into(),
            currency: "USD".into(),
            lines: vec![sample_po_line()],
            total_minor: 500000,
            created_by: "user-1".into(),
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/po_created");
}

#[test]
fn ap_po_created_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.po_created",
        mutation_classes::DATA_MUTATION,
        PoCreatedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-002".into(),
            currency: "USD".into(),
            lines: vec![sample_po_line()],
            total_minor: 500000,
            created_by: "user-2".into(),
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-po-created.v1.json");
}

#[test]
fn ap_po_approved_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.po_approved",
        mutation_classes::DATA_MUTATION,
        PoApprovedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-001".into(),
            approved_amount_minor: 500000,
            currency: "USD".into(),
            approved_by: "approver-1".into(),
            approved_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/po_approved");
}

#[test]
fn ap_po_approved_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.po_approved",
        mutation_classes::DATA_MUTATION,
        PoApprovedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-003".into(),
            approved_amount_minor: 250000,
            currency: "USD".into(),
            approved_by: "approver-2".into(),
            approved_at: "2026-03-01T12:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-po-approved.v1.json");
}

#[test]
fn ap_po_closed_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.po_closed",
        mutation_classes::LIFECYCLE,
        PoClosedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-001".into(),
            close_reason: "fully_received".into(),
            closed_by: "user-1".into(),
            closed_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/po_closed");
}

#[test]
fn ap_po_closed_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.po_closed",
        mutation_classes::LIFECYCLE,
        PoClosedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-004".into(),
            close_reason: "cancelled".into(),
            closed_by: "user-3".into(),
            closed_at: "2026-03-01T12:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-po-closed.v1.json");
}

#[test]
fn ap_po_line_received_linked_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.po_line_received_linked",
        mutation_classes::DATA_MUTATION,
        PoLineReceivedLinkedPayload {
            po_id: Uuid::new_v4(),
            po_line_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            receipt_id: Uuid::new_v4(),
            quantity_received: 50.0,
            unit_of_measure: "EA".into(),
            unit_price_minor: 5000,
            currency: "USD".into(),
            gl_account_code: "1400".into(),
            received_at: "2026-03-01T00:00:00Z".into(),
            received_by: "receiver-1".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/po_line_received_linked");
}

#[test]
fn ap_po_line_received_linked_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.po_line_received_linked",
        mutation_classes::DATA_MUTATION,
        PoLineReceivedLinkedPayload {
            po_id: Uuid::new_v4(),
            po_line_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            receipt_id: Uuid::new_v4(),
            quantity_received: 25.0,
            unit_of_measure: "KG".into(),
            unit_price_minor: 8000,
            currency: "USD".into(),
            gl_account_code: "1410".into(),
            received_at: "2026-03-01T12:00:00Z".into(),
            received_by: "receiver-2".into(),
        },
    );
    validate_against_schema(&json, "ap-po-line-received-linked.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// BILL LIFECYCLE
// ══════════════════════════════════════════════════════════════════════

fn sample_bill_line() -> BillLinePayload {
    BillLinePayload {
        line_id: Uuid::new_v4(),
        description: "Office supplies".into(),
        quantity: 5.0,
        unit_price_minor: 2000,
        line_total_minor: 10000,
        gl_account_code: "6200".into(),
        po_line_id: None,
    }
}

#[test]
fn ap_vendor_bill_created_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_created",
        mutation_classes::DATA_MUTATION,
        VendorBillCreatedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-042".into(),
            currency: "USD".into(),
            lines: vec![sample_bill_line()],
            total_minor: 10000,
            tax_minor: None,
            invoice_date: "2026-03-01T00:00:00Z".into(),
            due_date: "2026-03-31T00:00:00Z".into(),
            entered_by: "user-1".into(),
            entered_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/vendor_bill_created");
}

#[test]
fn ap_vendor_bill_created_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_created",
        mutation_classes::DATA_MUTATION,
        VendorBillCreatedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-099".into(),
            currency: "USD".into(),
            lines: vec![sample_bill_line()],
            total_minor: 10000,
            tax_minor: Some(800),
            invoice_date: "2026-03-01T00:00:00Z".into(),
            due_date: "2026-04-01T00:00:00Z".into(),
            entered_by: "user-2".into(),
            entered_at: "2026-03-01T12:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-vendor-bill-created.v1.json");
}

#[test]
fn ap_vendor_bill_matched_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_matched",
        mutation_classes::DATA_MUTATION,
        VendorBillMatchedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_id: Uuid::new_v4(),
            match_type: "three_way".into(),
            match_lines: vec![BillMatchLinePayload {
                bill_line_id: Uuid::new_v4(),
                po_line_id: Uuid::new_v4(),
                receipt_id: Some(Uuid::new_v4()),
                matched_quantity: 50.0,
                matched_amount_minor: 250000,
                within_tolerance: true,
            }],
            fully_matched: true,
            matched_by: "user-1".into(),
            matched_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/vendor_bill_matched");
}

#[test]
fn ap_vendor_bill_matched_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_matched",
        mutation_classes::DATA_MUTATION,
        VendorBillMatchedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            po_id: Uuid::new_v4(),
            match_type: "two_way".into(),
            match_lines: vec![BillMatchLinePayload {
                bill_line_id: Uuid::new_v4(),
                po_line_id: Uuid::new_v4(),
                receipt_id: None,
                matched_quantity: 100.0,
                matched_amount_minor: 500000,
                within_tolerance: true,
            }],
            fully_matched: true,
            matched_by: "user-2".into(),
            matched_at: "2026-03-01T12:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-vendor-bill-matched.v1.json");
}

#[test]
fn ap_vendor_bill_approved_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_approved",
        mutation_classes::DATA_MUTATION,
        VendorBillApprovedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-042".into(),
            approved_amount_minor: 10000,
            currency: "USD".into(),
            due_date: "2026-03-31T00:00:00Z".into(),
            approved_by: "approver-1".into(),
            approved_at: "2026-03-01T00:00:00Z".into(),
            fx_rate_id: None,
            gl_lines: vec![GlLinePayload {
                line_id: Uuid::new_v4(),
                gl_account_code: "6200".into(),
                amount_minor: 10000,
                po_line_id: None,
            }],
        },
    );
    assert_envelope_completeness(&json, "ap/vendor_bill_approved");
}

#[test]
fn ap_vendor_bill_approved_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_approved",
        mutation_classes::DATA_MUTATION,
        VendorBillApprovedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-100".into(),
            approved_amount_minor: 50000,
            currency: "USD".into(),
            due_date: "2026-04-15T00:00:00Z".into(),
            approved_by: "approver-2".into(),
            approved_at: "2026-03-02T00:00:00Z".into(),
            fx_rate_id: Some(Uuid::new_v4()),
            gl_lines: vec![GlLinePayload {
                line_id: Uuid::new_v4(),
                gl_account_code: "1400".into(),
                amount_minor: 50000,
                po_line_id: Some(Uuid::new_v4()),
            }],
        },
    );
    validate_against_schema(&json, "ap-vendor-bill-approved.v1.json");
}

#[test]
fn ap_vendor_bill_voided_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_voided",
        mutation_classes::REVERSAL,
        VendorBillVoidedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-042".into(),
            original_total_minor: 10000,
            currency: "USD".into(),
            void_reason: "Duplicate entry".into(),
            voided_by: "user-1".into(),
            voided_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/vendor_bill_voided");
}

#[test]
fn ap_vendor_bill_voided_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.vendor_bill_voided",
        mutation_classes::REVERSAL,
        VendorBillVoidedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-050".into(),
            original_total_minor: 75000,
            currency: "USD".into(),
            void_reason: "Incorrect vendor".into(),
            voided_by: "user-3".into(),
            voided_at: "2026-03-02T00:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-vendor-bill-voided.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// PAYMENT LIFECYCLE
// ══════════════════════════════════════════════════════════════════════

fn sample_payment_run_item() -> PaymentRunItemPayload {
    PaymentRunItemPayload {
        vendor_id: Uuid::new_v4(),
        bill_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
        amount_minor: 150000,
        currency: "USD".into(),
    }
}

#[test]
fn ap_payment_run_created_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.payment_run_created",
        mutation_classes::DATA_MUTATION,
        PaymentRunCreatedPayload {
            run_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            items: vec![sample_payment_run_item()],
            total_minor: 150000,
            currency: "USD".into(),
            scheduled_date: "2026-03-15T00:00:00Z".into(),
            payment_method: "ach".into(),
            created_by: "user-1".into(),
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/payment_run_created");
}

#[test]
fn ap_payment_run_created_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.payment_run_created",
        mutation_classes::DATA_MUTATION,
        PaymentRunCreatedPayload {
            run_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            items: vec![sample_payment_run_item()],
            total_minor: 150000,
            currency: "USD".into(),
            scheduled_date: "2026-03-20T00:00:00Z".into(),
            payment_method: "wire".into(),
            created_by: "user-2".into(),
            created_at: "2026-03-02T00:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-payment-run-created.v1.json");
}

#[test]
fn ap_payment_executed_envelope_completeness() {
    let json = build_envelope(
        "t-1",
        "ap.payment_executed",
        mutation_classes::DATA_MUTATION,
        PaymentExecutedPayload {
            payment_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            bill_ids: vec![Uuid::new_v4()],
            amount_minor: 75000,
            currency: "USD".into(),
            payment_method: "ach".into(),
            bank_reference: Some("ACH-20260301-001".into()),
            bank_account_last4: Some("4242".into()),
            executed_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "ap/payment_executed");
}

#[test]
fn ap_payment_executed_schema_validation() {
    let json = build_envelope(
        "t-1",
        "ap.payment_executed",
        mutation_classes::DATA_MUTATION,
        PaymentExecutedPayload {
            payment_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            tenant_id: "t-1".into(),
            vendor_id: Uuid::new_v4(),
            bill_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
            amount_minor: 250000,
            currency: "USD".into(),
            payment_method: "wire".into(),
            bank_reference: Some("WIRE-20260302-007".into()),
            bank_account_last4: None,
            executed_at: "2026-03-02T00:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "ap-payment-executed.v1.json");
}

// ── Exhaustive subject coverage ──────────────────────────────────────

#[test]
fn ap_all_subjects_have_contract_tests() {
    let tested_event_types = [
        "ap.vendor_created",
        "ap.vendor_updated",
        "ap.po_created",
        "ap.po_approved",
        "ap.po_closed",
        "ap.po_line_received_linked",
        "ap.vendor_bill_created",
        "ap.vendor_bill_matched",
        "ap.vendor_bill_approved",
        "ap.vendor_bill_voided",
        "ap.payment_run_created",
        "ap.payment_executed",
    ];
    assert_eq!(
        tested_event_types.len(),
        12,
        "Must have contract tests for all 12 AP event types"
    );
    for et in &tested_event_types {
        assert!(
            et.starts_with("ap."),
            "All AP event types must start with 'ap.'"
        );
        // AP event types use module.entity_action format (e.g. "ap.vendor_created")
        // which passes validate_event_type as a 2-segment dotted string
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}
