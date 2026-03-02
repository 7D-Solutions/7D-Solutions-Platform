//! Credit note and write-off event contracts:
//! ar.credit_note_issued, ar.invoice_written_off

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_REVERSAL};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A credit note was formally issued against an invoice
pub const EVENT_TYPE_CREDIT_NOTE_ISSUED: &str = "ar.credit_note_issued";

/// An invoice was written off as uncollectable bad debt
pub const EVENT_TYPE_INVOICE_WRITTEN_OFF: &str = "ar.invoice_written_off";

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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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
}
