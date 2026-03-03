//! Progress billing / milestone invoicing event contracts:
//! ar.milestone_invoice_created

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A milestone invoice was created against a progress billing contract
pub const EVENT_TYPE_MILESTONE_INVOICE_CREATED: &str = "ar.milestone_invoice_created";

// ============================================================================
// Payload: ar.milestone_invoice_created
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneInvoiceCreatedPayload {
    pub contract_id: Uuid,
    pub milestone_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub invoice_id: i32,
    pub amount_minor: i64,
    pub currency: String,
    pub milestone_name: String,
    pub percentage: i32,
    pub cumulative_billed_minor: i64,
    pub contract_total_minor: i64,
    pub created_at: DateTime<Utc>,
}

pub fn build_milestone_invoice_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: MilestoneInvoiceCreatedPayload,
) -> EventEnvelope<MilestoneInvoiceCreatedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_MILESTONE_INVOICE_CREATED.to_string(),
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

    #[test]
    fn milestone_invoice_created_envelope_has_correct_metadata() {
        let payload = MilestoneInvoiceCreatedPayload {
            contract_id: Uuid::new_v4(),
            milestone_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            invoice_id: 42,
            amount_minor: 50000,
            currency: "usd".to_string(),
            milestone_name: "Foundation Complete".to_string(),
            percentage: 30,
            cumulative_billed_minor: 50000,
            contract_total_minor: 200000,
            created_at: Utc::now(),
        };
        let envelope = build_milestone_invoice_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_MILESTONE_INVOICE_CREATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }
}
