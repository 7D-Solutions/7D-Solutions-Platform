//! AR event type constants and payload structs
//!
//! Defines the canonical event contracts for AR's monetization events:
//! - ar.usage_captured       (metered usage recorded)
//! - ar.usage_invoiced       (usage billed on an invoice)
//! - ar.credit_note_issued   (credit note issued against an invoice)
//! - ar.invoice_written_off  (invoice written off as bad debt)
//! - ar.ar_aging_updated     (AR aging projection updated)
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0" (stable for this event version)
//! - mutation_class: per event (DATA_MUTATION or REVERSAL)
//! - correlation_id / causation_id: caller-supplied for tracing
//! - event_id: caller-supplied for idempotency (deterministic from business key)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// Metered usage was captured for billing (idempotent per usage_id)
pub const EVENT_TYPE_USAGE_CAPTURED: &str = "ar.usage_captured";

/// Captured usage was billed on an invoice line item
pub const EVENT_TYPE_USAGE_INVOICED: &str = "ar.usage_invoiced";

/// A credit note was formally issued against an invoice
pub const EVENT_TYPE_CREDIT_NOTE_ISSUED: &str = "ar.credit_note_issued";

/// An invoice was written off as uncollectable bad debt
pub const EVENT_TYPE_INVOICE_WRITTEN_OFF: &str = "ar.invoice_written_off";

/// AR aging buckets were updated (projection refresh)
pub const EVENT_TYPE_AR_AGING_UPDATED: &str = "ar.ar_aging_updated";

// ============================================================================
// Schema Version
// ============================================================================

/// Schema version for all AR monetization event payloads (v1)
pub const AR_EVENT_SCHEMA_VERSION: &str = "1.0.0";

// ============================================================================
// Mutation Classes
// ============================================================================

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

/// REVERSAL: compensates for a prior DATA_MUTATION (write-off, void)
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";

// ============================================================================
// Payload: ar.usage_captured
// ============================================================================

/// Payload for ar.usage_captured
///
/// Emitted when metered usage is recorded for a tenant/customer.
/// Idempotency: caller MUST supply a deterministic event_id derived from usage_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageCapturedPayload {
    /// Stable business key for this usage record (idempotency anchor)
    pub usage_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    /// Metric being measured (e.g. "api_calls", "gb_storage")
    pub metric_name: String,
    /// Quantity consumed
    pub quantity: f64,
    /// Unit for the quantity (e.g. "calls", "GB")
    pub unit: String,
    /// Start of the usage measurement period
    pub period_start: DateTime<Utc>,
    /// End of the usage measurement period
    pub period_end: DateTime<Utc>,
    /// Subscription this usage belongs to (if applicable)
    pub subscription_id: Option<Uuid>,
    pub captured_at: DateTime<Utc>,
}

/// Build an envelope for ar.usage_captured
///
/// mutation_class: DATA_MUTATION (new usage record)
pub fn build_usage_captured_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: UsageCapturedPayload,
) -> EventEnvelope<UsageCapturedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_USAGE_CAPTURED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.usage_invoiced
// ============================================================================

/// Payload for ar.usage_invoiced
///
/// Emitted when captured usage is billed as a line item on an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInvoicedPayload {
    pub usage_id: Uuid,
    pub invoice_id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub metric_name: String,
    pub quantity: f64,
    pub unit: String,
    /// Unit price in minor currency units (e.g. cents)
    pub unit_price_minor: i64,
    /// Total billed amount in minor currency units
    pub total_minor: i64,
    pub currency: String,
    pub invoiced_at: DateTime<Utc>,
}

/// Build an envelope for ar.usage_invoiced
///
/// mutation_class: DATA_MUTATION (new invoice line item)
pub fn build_usage_invoiced_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: UsageInvoicedPayload,
) -> EventEnvelope<UsageInvoicedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_USAGE_INVOICED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.credit_note_issued
// ============================================================================

/// Payload for ar.credit_note_issued
///
/// Emitted when a formal credit note is issued against an invoice.
/// Credit notes reduce the amount owed without voiding the original invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditNoteIssuedPayload {
    /// Stable business key for this credit note
    pub credit_note_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    /// Invoice this credit note is applied against
    pub invoice_id: String,
    /// Credit amount in minor currency units (positive = reduces balance)
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    /// Optional reference to original usage or line item
    pub reference_id: Option<String>,
    pub issued_at: DateTime<Utc>,
}

/// Build an envelope for ar.credit_note_issued
///
/// mutation_class: DATA_MUTATION (creates a new credit note artifact)
pub fn build_credit_note_issued_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: CreditNoteIssuedPayload,
) -> EventEnvelope<CreditNoteIssuedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CREDIT_NOTE_ISSUED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.invoice_written_off
// ============================================================================

/// Payload for ar.invoice_written_off
///
/// Emitted when an invoice is written off as uncollectable bad debt.
/// Write-off is a financial reversal of the receivable — the debt is forgiven.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceWrittenOffPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Outstanding amount written off in minor currency units
    pub written_off_amount_minor: i64,
    pub currency: String,
    /// Reason for write-off (e.g. "uncollectable", "bankruptcy", "dispute_settled")
    pub reason: String,
    /// Actor who authorized the write-off
    pub authorized_by: Option<String>,
    pub written_off_at: DateTime<Utc>,
}

/// Build an envelope for ar.invoice_written_off
///
/// mutation_class: REVERSAL (compensates for the original receivable)
pub fn build_invoice_written_off_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceWrittenOffPayload,
) -> EventEnvelope<InvoiceWrittenOffPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_WRITTEN_OFF.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.ar_aging_updated
// ============================================================================

/// Summary of outstanding balances by aging bucket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgingBuckets {
    /// Invoices not yet due (amount_minor)
    pub current_minor: i64,
    /// 1–30 days overdue
    pub days_1_30_minor: i64,
    /// 31–60 days overdue
    pub days_31_60_minor: i64,
    /// 61–90 days overdue
    pub days_61_90_minor: i64,
    /// Over 90 days overdue
    pub days_over_90_minor: i64,
    /// Total outstanding (sum of all buckets)
    pub total_outstanding_minor: i64,
    pub currency: String,
}

/// Payload for ar.ar_aging_updated
///
/// Emitted when the AR aging projection is refreshed for a tenant.
/// Captures point-in-time aging buckets for the tenant's receivables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArAgingUpdatedPayload {
    pub tenant_id: String,
    /// Invoice count included in this snapshot
    pub invoice_count: i64,
    pub buckets: AgingBuckets,
    /// Timestamp of the aging calculation (as-of date)
    pub calculated_at: DateTime<Utc>,
}

/// Build an envelope for ar.ar_aging_updated
///
/// mutation_class: DATA_MUTATION (creates/updates an aging projection record)
pub fn build_ar_aging_updated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ArAgingUpdatedPayload,
) -> EventEnvelope<ArAgingUpdatedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_AR_AGING_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_usage_captured() -> UsageCapturedPayload {
        UsageCapturedPayload {
            usage_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            metric_name: "api_calls".to_string(),
            quantity: 1500.0,
            unit: "calls".to_string(),
            period_start: Utc::now(),
            period_end: Utc::now(),
            subscription_id: None,
            captured_at: Utc::now(),
        }
    }

    #[test]
    fn usage_captured_envelope_has_correct_type_and_class() {
        let envelope = build_usage_captured_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            sample_usage_captured(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_USAGE_CAPTURED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn usage_invoiced_envelope_has_correct_metadata() {
        let payload = UsageInvoicedPayload {
            usage_id: Uuid::new_v4(),
            invoice_id: "inv-42".to_string(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            metric_name: "api_calls".to_string(),
            quantity: 1500.0,
            unit: "calls".to_string(),
            unit_price_minor: 10,
            total_minor: 15000,
            currency: "usd".to_string(),
            invoiced_at: Utc::now(),
        };
        let envelope = build_usage_invoiced_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_USAGE_INVOICED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
    }

    #[test]
    fn credit_note_envelope_has_correct_metadata() {
        let payload = CreditNoteIssuedPayload {
            credit_note_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            invoice_id: "inv-99".to_string(),
            amount_minor: 5000,
            currency: "usd".to_string(),
            reason: "service_credit".to_string(),
            reference_id: None,
            issued_at: Utc::now(),
        };
        let envelope = build_credit_note_issued_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CREDIT_NOTE_ISSUED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn write_off_envelope_has_reversal_mutation_class() {
        let payload = InvoiceWrittenOffPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-77".to_string(),
            customer_id: "cust-1".to_string(),
            written_off_amount_minor: 20000,
            currency: "usd".to_string(),
            reason: "uncollectable".to_string(),
            authorized_by: Some("admin@tenant.local".to_string()),
            written_off_at: Utc::now(),
        };
        let envelope = build_invoice_written_off_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_WRITTEN_OFF);
        // Write-off is a REVERSAL of the receivable
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_REVERSAL)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn aging_updated_envelope_has_correct_metadata() {
        let payload = ArAgingUpdatedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_count: 12,
            buckets: AgingBuckets {
                current_minor: 100000,
                days_1_30_minor: 50000,
                days_31_60_minor: 20000,
                days_61_90_minor: 5000,
                days_over_90_minor: 2000,
                total_outstanding_minor: 177000,
                currency: "usd".to_string(),
            },
            calculated_at: Utc::now(),
        };
        let envelope = build_ar_aging_updated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_AR_AGING_UPDATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn all_envelopes_have_stable_schema_version() {
        assert_eq!(AR_EVENT_SCHEMA_VERSION, "1.0.0");
    }

    #[test]
    fn all_event_type_constants_use_ar_prefix() {
        assert!(EVENT_TYPE_USAGE_CAPTURED.starts_with("ar."));
        assert!(EVENT_TYPE_USAGE_INVOICED.starts_with("ar."));
        assert!(EVENT_TYPE_CREDIT_NOTE_ISSUED.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_WRITTEN_OFF.starts_with("ar."));
        assert!(EVENT_TYPE_AR_AGING_UPDATED.starts_with("ar."));
    }

    #[test]
    fn usage_captured_payload_serializes_correctly() {
        let payload = sample_usage_captured();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("usage_id"));
        assert!(json.contains("metric_name"));
        assert!(json.contains("api_calls"));
    }

    #[test]
    fn aging_buckets_total_fields_present() {
        let buckets = AgingBuckets {
            current_minor: 1000,
            days_1_30_minor: 500,
            days_31_60_minor: 200,
            days_61_90_minor: 100,
            days_over_90_minor: 50,
            total_outstanding_minor: 1850,
            currency: "usd".to_string(),
        };
        let json = serde_json::to_string(&buckets).expect("serialization failed");
        assert!(json.contains("total_outstanding_minor"));
        assert!(json.contains("days_over_90_minor"));
    }
}
