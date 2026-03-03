//! Payment terms event contract:
//!   ap.payment_terms_created

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AP_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};
use crate::events::envelope::{create_ap_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A new set of payment terms was created
pub const EVENT_TYPE_PAYMENT_TERMS_CREATED: &str = "ap.payment_terms_created";

// ============================================================================
// Payload: ap.payment_terms_created
// ============================================================================

/// Payload for ap.payment_terms_created
///
/// Self-contained: all term details included — no mutable state reads needed
/// at replay time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentTermsCreatedPayload {
    pub term_id: Uuid,
    pub tenant_id: String,
    pub term_code: String,
    pub description: String,
    pub days_due: i32,
    pub discount_pct: f64,
    pub discount_days: i32,
    pub created_at: DateTime<Utc>,
}

/// Build an envelope for ap.payment_terms_created
pub fn build_payment_terms_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PaymentTermsCreatedPayload,
) -> EventEnvelope<PaymentTermsCreatedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PAYMENT_TERMS_CREATED.to_string(),
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

    #[test]
    fn payment_terms_created_envelope_has_correct_metadata() {
        let payload = PaymentTermsCreatedPayload {
            term_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            term_code: "NET30".to_string(),
            description: "Net 30 days".to_string(),
            days_due: 30,
            discount_pct: 0.0,
            discount_days: 0,
            created_at: Utc::now(),
        };
        let envelope = build_payment_terms_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_PAYMENT_TERMS_CREATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.source_module, "ap");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn event_type_uses_ap_prefix() {
        assert!(EVENT_TYPE_PAYMENT_TERMS_CREATED.starts_with("ap."));
    }
}
