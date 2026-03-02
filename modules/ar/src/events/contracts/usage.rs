//! Usage event contracts: ar.usage_captured, ar.usage_invoiced

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// Metered usage was captured for billing (idempotent per usage_id)
pub const EVENT_TYPE_USAGE_CAPTURED: &str = "ar.usage_captured";

/// Captured usage was billed on an invoice line item
pub const EVENT_TYPE_USAGE_INVOICED: &str = "ar.usage_invoiced";

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
    fn usage_captured_payload_serializes_correctly() {
        let payload = sample_usage_captured();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("usage_id"));
        assert!(json.contains("metric_name"));
        assert!(json.contains("api_calls"));
    }
}
