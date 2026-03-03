//! Event contract: edi_transaction.status_changed

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};

pub const EVENT_TYPE_EDI_TRANSACTION_STATUS_CHANGED: &str = "edi_transaction.status_changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdiTransactionStatusChangedPayload {
    pub transaction_id: Uuid,
    pub tenant_id: String,
    pub previous_status: String,
    pub new_status: String,
    pub error_details: Option<String>,
    pub changed_at: DateTime<Utc>,
}

pub fn build_edi_transaction_status_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: EdiTransactionStatusChangedPayload,
) -> EventEnvelope<EdiTransactionStatusChangedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_EDI_TRANSACTION_STATUS_CHANGED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edi_transaction_status_changed_envelope_metadata() {
        let payload = EdiTransactionStatusChangedPayload {
            transaction_id: Uuid::new_v4(),
            tenant_id: "t-1".to_string(),
            previous_status: "ingested".to_string(),
            new_status: "parsed".to_string(),
            error_details: None,
            changed_at: Utc::now(),
        };
        let env = build_edi_transaction_status_changed_envelope(
            Uuid::new_v4(),
            "t-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_EDI_TRANSACTION_STATUS_CHANGED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
    }
}
