//! Event contract: edi_transaction.created

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};

pub const EVENT_TYPE_EDI_TRANSACTION_CREATED: &str = "edi_transaction.created";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdiTransactionCreatedPayload {
    pub transaction_id: Uuid,
    pub tenant_id: String,
    pub transaction_type: String,
    pub version: String,
    pub direction: String,
    pub validation_status: String,
    pub created_at: DateTime<Utc>,
}

pub fn build_edi_transaction_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: EdiTransactionCreatedPayload,
) -> EventEnvelope<EdiTransactionCreatedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_EDI_TRANSACTION_CREATED.to_string(),
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
    fn edi_transaction_created_envelope_metadata() {
        let payload = EdiTransactionCreatedPayload {
            transaction_id: Uuid::new_v4(),
            tenant_id: "t-1".to_string(),
            transaction_type: "850".to_string(),
            version: "004010".to_string(),
            direction: "inbound".to_string(),
            validation_status: "ingested".to_string(),
            created_at: Utc::now(),
        };
        let env = build_edi_transaction_created_envelope(
            Uuid::new_v4(),
            "t-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_EDI_TRANSACTION_CREATED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
    }
}
