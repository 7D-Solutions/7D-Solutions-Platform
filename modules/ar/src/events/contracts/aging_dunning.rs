//! Aging and dunning event contracts:
//! ar.ar_aging_updated, ar.dunning_state_changed, ar.invoice_suspended

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// AR aging buckets were updated (projection refresh)
pub const EVENT_TYPE_AR_AGING_UPDATED: &str = "ar.ar_aging_updated";

/// Dunning state machine transitioned (e.g. pending → warned → suspended → resolved)
pub const EVENT_TYPE_DUNNING_STATE_CHANGED: &str = "ar.dunning_state_changed";

/// Invoice suspended due to non-payment escalation
pub const EVENT_TYPE_INVOICE_SUSPENDED: &str = "ar.invoice_suspended";

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
// Payload: ar.dunning_state_changed
// ============================================================================

/// Dunning state machine values
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DunningState {
    Pending,
    Warned,
    Escalated,
    Suspended,
    Resolved,
    WrittenOff,
}

/// Payload for ar.dunning_state_changed
///
/// Emitted when the dunning state machine transitions for an invoice or customer.
/// Idempotency: caller MUST supply a deterministic event_id derived from (invoice_id, to_state, occurred_at).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DunningStateChangedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Previous dunning state (None if this is the initial state)
    pub from_state: Option<DunningState>,
    /// New dunning state after transition
    pub to_state: DunningState,
    /// Human-readable reason for the transition
    pub reason: String,
    /// Which dunning attempt number triggered this (1-indexed)
    pub attempt_number: i32,
    /// Next retry scheduled at (None if terminal state)
    pub next_retry_at: Option<DateTime<Utc>>,
    pub transitioned_at: DateTime<Utc>,
}

/// Build an envelope for ar.dunning_state_changed
///
/// mutation_class: LIFECYCLE (state machine transition)
pub fn build_dunning_state_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: DunningStateChangedPayload,
) -> EventEnvelope<DunningStateChangedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_DUNNING_STATE_CHANGED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.invoice_suspended
// ============================================================================

/// Payload for ar.invoice_suspended
///
/// Emitted when an invoice is formally suspended due to dunning escalation.
/// Suspension may trigger service interruption upstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSuspendedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Outstanding balance at the time of suspension (minor currency units)
    pub outstanding_minor: i64,
    pub currency: String,
    /// Which dunning attempt number triggered suspension
    pub dunning_attempt: i32,
    /// Reason for suspension
    pub reason: String,
    /// Suspension may be lifted if payment received before this date
    pub grace_period_ends_at: Option<DateTime<Utc>>,
    pub suspended_at: DateTime<Utc>,
}

/// Build an envelope for ar.invoice_suspended
///
/// mutation_class: LIFECYCLE (invoice enters suspended lifecycle state)
pub fn build_invoice_suspended_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceSuspendedPayload,
) -> EventEnvelope<InvoiceSuspendedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_SUSPENDED.to_string(),
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

    #[test]
    fn dunning_state_changed_envelope_has_lifecycle_class() {
        let payload = DunningStateChangedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-100".to_string(),
            customer_id: "cust-1".to_string(),
            from_state: Some(DunningState::Pending),
            to_state: DunningState::Warned,
            reason: "first_overdue_notice".to_string(),
            attempt_number: 1,
            next_retry_at: Some(Utc::now()),
            transitioned_at: Utc::now(),
        };
        let envelope = build_dunning_state_changed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_DUNNING_STATE_CHANGED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn invoice_suspended_envelope_has_lifecycle_class() {
        let payload = InvoiceSuspendedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-101".to_string(),
            customer_id: "cust-1".to_string(),
            outstanding_minor: 50000,
            currency: "usd".to_string(),
            dunning_attempt: 3,
            reason: "max_attempts_exceeded".to_string(),
            grace_period_ends_at: None,
            suspended_at: Utc::now(),
        };
        let envelope = build_invoice_suspended_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("cause-dunning".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_SUSPENDED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-dunning"));
    }

    #[test]
    fn dunning_state_serializes_as_snake_case() {
        let state = DunningState::WrittenOff;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"written_off\"");
    }

    #[test]
    fn aging_dunning_event_type_constants_use_ar_prefix() {
        assert!(EVENT_TYPE_AR_AGING_UPDATED.starts_with("ar."));
        assert!(EVENT_TYPE_DUNNING_STATE_CHANGED.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_SUSPENDED.starts_with("ar."));
    }
}
