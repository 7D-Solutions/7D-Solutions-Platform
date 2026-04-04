//! Invoice lifecycle event contracts:
//! ar.invoice_opened (on invoice INSERT), ar.invoice_paid (on status → paid)
//!
//! These events feed the cash flow forecasting module (Phase 51).
//! The publisher routes them to NATS subjects:
//!   ar.events.ar.invoice_opened
//!   ar.events.ar.invoice_paid

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// An invoice was inserted into ar_invoices
pub const EVENT_TYPE_INVOICE_OPENED: &str = "ar.invoice_opened";

/// An invoice transitioned to status='attempting'
pub const EVENT_TYPE_INVOICE_ATTEMPTING: &str = "ar.invoice_attempting";

/// An invoice transitioned to status='paid'
pub const EVENT_TYPE_INVOICE_PAID: &str = "ar.invoice_paid";

/// An invoice transitioned to status='failed_final'
pub const EVENT_TYPE_INVOICE_FAILED_FINAL: &str = "ar.invoice_failed_final";

/// An invoice transitioned to status='void'
pub const EVENT_TYPE_INVOICE_VOID: &str = "ar.invoice_void";

// ============================================================================
// Payload (shared shape for both events)
// ============================================================================

/// Payload for ar.invoice_opened and ar.invoice_paid.
///
/// `paid_at` is None for invoice_opened; non-None for invoice_paid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLifecyclePayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub app_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub created_at: NaiveDateTime,
    pub due_at: Option<NaiveDateTime>,
    pub paid_at: Option<NaiveDateTime>,
}

// ============================================================================
// Envelope builders
// ============================================================================

/// Build envelope for ar.invoice_opened (mutation_class: LIFECYCLE)
pub fn build_invoice_opened_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceLifecyclePayload,
) -> EventEnvelope<InvoiceLifecyclePayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_OPENED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

/// Build envelope for ar.invoice_attempting (mutation_class: LIFECYCLE)
pub fn build_invoice_attempting_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceLifecyclePayload,
) -> EventEnvelope<InvoiceLifecyclePayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_ATTEMPTING.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

/// Build envelope for ar.invoice_paid (mutation_class: LIFECYCLE)
pub fn build_invoice_paid_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceLifecyclePayload,
) -> EventEnvelope<InvoiceLifecyclePayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_PAID.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

/// Build envelope for ar.invoice_failed_final (mutation_class: LIFECYCLE)
pub fn build_invoice_failed_final_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceLifecyclePayload,
) -> EventEnvelope<InvoiceLifecyclePayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_FAILED_FINAL.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

/// Build envelope for ar.invoice_void (mutation_class: LIFECYCLE)
pub fn build_invoice_void_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceLifecyclePayload,
) -> EventEnvelope<InvoiceLifecyclePayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_VOID.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
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

    fn sample_payload(paid_at: Option<NaiveDateTime>) -> InvoiceLifecyclePayload {
        InvoiceLifecyclePayload {
            invoice_id: "42".to_string(),
            customer_id: "7".to_string(),
            app_id: "test-app".to_string(),
            amount_cents: 5000,
            currency: "usd".to_string(),
            created_at: Utc::now().naive_utc(),
            due_at: None,
            paid_at,
        }
    }

    #[test]
    fn invoice_opened_envelope_has_correct_metadata() {
        let envelope = build_invoice_opened_envelope(
            Uuid::new_v4(),
            "test-app".to_string(),
            "corr-1".to_string(),
            None,
            sample_payload(None),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_OPENED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
        assert!(envelope.payload.paid_at.is_none());
    }

    #[test]
    fn invoice_paid_envelope_has_correct_metadata() {
        let paid_at = Some(Utc::now().naive_utc());
        let envelope = build_invoice_paid_envelope(
            Uuid::new_v4(),
            "test-app".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            sample_payload(paid_at),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_PAID);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
        assert!(envelope.payload.paid_at.is_some());
    }

    #[test]
    fn invoice_attempting_envelope_has_correct_metadata() {
        let envelope = build_invoice_attempting_envelope(
            Uuid::new_v4(),
            "test-app".to_string(),
            "corr-3".to_string(),
            None,
            sample_payload(None),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_ATTEMPTING);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn invoice_failed_final_envelope_has_correct_metadata() {
        let envelope = build_invoice_failed_final_envelope(
            Uuid::new_v4(),
            "test-app".to_string(),
            "corr-4".to_string(),
            None,
            sample_payload(None),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_FAILED_FINAL);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
    }

    #[test]
    fn invoice_void_envelope_has_correct_metadata() {
        let envelope = build_invoice_void_envelope(
            Uuid::new_v4(),
            "test-app".to_string(),
            "corr-5".to_string(),
            None,
            sample_payload(None),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_VOID);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
    }

    #[test]
    fn event_type_constants_use_ar_prefix() {
        assert!(EVENT_TYPE_INVOICE_OPENED.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_ATTEMPTING.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_PAID.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_FAILED_FINAL.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_VOID.starts_with("ar."));
    }
}
