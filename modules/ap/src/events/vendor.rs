//! Vendor event contracts: ap.vendor_created, ap.vendor_updated

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_ap_envelope, EventEnvelope};
use super::{AP_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A new vendor was registered in the AP module
pub const EVENT_TYPE_VENDOR_CREATED: &str = "ap.vendor_created";

/// A vendor's attributes were updated
pub const EVENT_TYPE_VENDOR_UPDATED: &str = "ap.vendor_updated";

// ============================================================================
// Payload: ap.vendor_created
// ============================================================================

/// Payload for ap.vendor_created
///
/// Emitted when a new vendor is registered.
/// Self-contained: includes all fields needed to reconstruct the vendor record
/// without reading current state at replay time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorCreatedPayload {
    /// Stable vendor identifier (idempotency anchor)
    pub vendor_id: Uuid,
    pub tenant_id: String,
    /// Legal name of the vendor
    pub name: String,
    /// Tax identification number (EIN, VAT, etc.)
    pub tax_id: Option<String>,
    /// ISO 4217 currency code for payables (e.g. "USD")
    pub currency: String,
    /// Net payment terms in days (e.g. 30 for Net-30)
    pub payment_terms_days: i32,
    /// Preferred payment method (e.g. "ach", "wire", "check")
    pub payment_method: Option<String>,
    /// Remittance email address
    pub remittance_email: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Build an envelope for ap.vendor_created
pub fn build_vendor_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorCreatedPayload,
) -> EventEnvelope<VendorCreatedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VENDOR_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.vendor_updated
// ============================================================================

/// Payload for ap.vendor_updated
///
/// Emitted when vendor attributes change. Carries the full new state of the
/// changed fields (not a diff) so replaying produces the same outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorUpdatedPayload {
    pub vendor_id: Uuid,
    pub tenant_id: String,
    /// Updated legal name (None = unchanged)
    pub name: Option<String>,
    /// Updated tax ID (None = unchanged)
    pub tax_id: Option<String>,
    /// Updated currency (None = unchanged)
    pub currency: Option<String>,
    /// Updated payment terms in days (None = unchanged)
    pub payment_terms_days: Option<i32>,
    /// Updated payment method (None = unchanged)
    pub payment_method: Option<String>,
    /// Updated remittance email (None = unchanged)
    pub remittance_email: Option<String>,
    /// Actor who made the change
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

/// Build an envelope for ap.vendor_updated
pub fn build_vendor_updated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorUpdatedPayload,
) -> EventEnvelope<VendorUpdatedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VENDOR_UPDATED.to_string(),
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

    fn sample_vendor_created() -> VendorCreatedPayload {
        VendorCreatedPayload {
            vendor_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            name: "Acme Supplies LLC".to_string(),
            tax_id: Some("12-3456789".to_string()),
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: Some("ach".to_string()),
            remittance_email: Some("ap@acme.example".to_string()),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn vendor_created_envelope_has_correct_metadata() {
        let envelope = build_vendor_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            sample_vendor_created(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_VENDOR_CREATED);
        assert_eq!(envelope.mutation_class.as_deref(), Some(MUTATION_CLASS_DATA_MUTATION));
        assert_eq!(envelope.schema_version, AP_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ap");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn vendor_updated_envelope_has_correct_metadata() {
        let payload = VendorUpdatedPayload {
            vendor_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            name: Some("Acme Supplies Inc".to_string()),
            tax_id: None,
            currency: None,
            payment_terms_days: Some(45),
            payment_method: None,
            remittance_email: None,
            updated_by: "user-99".to_string(),
            updated_at: Utc::now(),
        };
        let envelope = build_vendor_updated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_VENDOR_UPDATED);
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
        assert_eq!(envelope.source_module, "ap");
    }

    #[test]
    fn vendor_created_payload_serializes_correctly() {
        let payload = sample_vendor_created();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("vendor_id"));
        assert!(json.contains("Acme Supplies LLC"));
        assert!(json.contains("payment_terms_days"));
    }
}
