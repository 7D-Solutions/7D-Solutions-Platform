//! AP payment event contracts:
//!   ap.payment_run_created, ap.payment_executed

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_ap_envelope, EventEnvelope};
use super::{AP_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constants
// ============================================================================

/// An AP payment run was created (batches vendor payments for processing)
pub const EVENT_TYPE_AP_PAYMENT_RUN_CREATED: &str = "ap.payment_run_created";

/// A single vendor payment was executed as part of a payment run
pub const EVENT_TYPE_AP_PAYMENT_EXECUTED: &str = "ap.payment_executed";

// ============================================================================
// Shared types
// ============================================================================

/// A vendor payment item within a payment run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRunItem {
    pub vendor_id: Uuid,
    /// Bills being paid in this item
    pub bill_ids: Vec<Uuid>,
    /// Total to pay this vendor in this run (minor currency units)
    pub amount_minor: i64,
    pub currency: String,
}

// ============================================================================
// Payload: ap.payment_run_created
// ============================================================================

/// Payload for ap.payment_run_created
///
/// Emitted when a batch of vendor payments is queued for execution.
/// Self-contained: includes all vendor/bill associations and amounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApPaymentRunCreatedPayload {
    pub run_id: Uuid,
    pub tenant_id: String,
    /// Items in this run — one per vendor
    pub items: Vec<PaymentRunItem>,
    /// Total across all items (minor currency units, single currency runs)
    pub total_minor: i64,
    pub currency: String,
    /// Scheduled execution date
    pub scheduled_date: DateTime<Utc>,
    /// Payment method for all items in this run (e.g. "ach", "wire", "check")
    pub payment_method: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

/// Build an envelope for ap.payment_run_created
pub fn build_ap_payment_run_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ApPaymentRunCreatedPayload,
) -> EventEnvelope<ApPaymentRunCreatedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_AP_PAYMENT_RUN_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.payment_executed
// ============================================================================

/// Payload for ap.payment_executed
///
/// Emitted per-vendor when a payment is sent. Self-contained: includes
/// run reference, bill IDs paid, payment method, bank reference, and amounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApPaymentExecutedPayload {
    pub payment_id: Uuid,
    pub run_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    /// Bills settled by this payment
    pub bill_ids: Vec<Uuid>,
    /// Amount paid in minor currency units
    pub amount_minor: i64,
    pub currency: String,
    /// Payment method used (e.g. "ach", "wire", "check")
    pub payment_method: String,
    /// Bank or payment processor reference number
    pub bank_reference: Option<String>,
    /// Bank account last 4 digits (for audit, non-sensitive)
    pub bank_account_last4: Option<String>,
    pub executed_at: DateTime<Utc>,
}

/// Build an envelope for ap.payment_executed
pub fn build_ap_payment_executed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ApPaymentExecutedPayload,
) -> EventEnvelope<ApPaymentExecutedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_AP_PAYMENT_EXECUTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run_items() -> Vec<PaymentRunItem> {
        vec![PaymentRunItem {
            vendor_id: Uuid::new_v4(),
            bill_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
            amount_minor: 150000,
            currency: "USD".to_string(),
        }]
    }

    #[test]
    fn payment_run_created_envelope_has_correct_metadata() {
        let payload = ApPaymentRunCreatedPayload {
            run_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            items: sample_run_items(),
            total_minor: 150000,
            currency: "USD".to_string(),
            scheduled_date: Utc::now(),
            payment_method: "ach".to_string(),
            created_by: "user-1".to_string(),
            created_at: Utc::now(),
        };
        let envelope = build_ap_payment_run_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_AP_PAYMENT_RUN_CREATED);
        assert_eq!(envelope.mutation_class.as_deref(), Some(MUTATION_CLASS_DATA_MUTATION));
        assert_eq!(envelope.schema_version, AP_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ap");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn payment_executed_envelope_has_correct_metadata() {
        let payload = ApPaymentExecutedPayload {
            payment_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            vendor_id: Uuid::new_v4(),
            bill_ids: vec![Uuid::new_v4()],
            amount_minor: 75000,
            currency: "USD".to_string(),
            payment_method: "ach".to_string(),
            bank_reference: Some("ACH-20260218-001".to_string()),
            bank_account_last4: Some("4242".to_string()),
            executed_at: Utc::now(),
        };
        let envelope = build_ap_payment_executed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("run-cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_AP_PAYMENT_EXECUTED);
        assert_eq!(envelope.causation_id.as_deref(), Some("run-cause-1"));
        assert_eq!(envelope.source_module, "ap");
    }

    #[test]
    fn payment_run_payload_serializes_bill_ids() {
        let bill_id = Uuid::new_v4();
        let item = PaymentRunItem {
            vendor_id: Uuid::new_v4(),
            bill_ids: vec![bill_id],
            amount_minor: 10000,
            currency: "USD".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("bill_ids"));
        assert!(json.contains(&bill_id.to_string()));
    }
}
