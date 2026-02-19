//! Party event contracts: party.created, party.updated, party.deactivated

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_party_envelope, EventEnvelope};
use super::{MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE, PARTY_EVENT_SCHEMA_VERSION};

// ============================================================================
// Event Type Constants
// ============================================================================

pub const EVENT_TYPE_PARTY_CREATED: &str = "party.created";
pub const EVENT_TYPE_PARTY_UPDATED: &str = "party.updated";
pub const EVENT_TYPE_PARTY_DEACTIVATED: &str = "party.deactivated";

// ============================================================================
// Payload: party.created
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyCreatedPayload {
    pub party_id: Uuid,
    pub app_id: String,
    pub party_type: String,
    pub display_name: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn build_party_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PartyCreatedPayload,
) -> EventEnvelope<PartyCreatedPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_PARTY_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: party.updated
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyUpdatedPayload {
    pub party_id: Uuid,
    pub app_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

pub fn build_party_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PartyUpdatedPayload,
) -> EventEnvelope<PartyUpdatedPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_PARTY_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: party.deactivated
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyDeactivatedPayload {
    pub party_id: Uuid,
    pub app_id: String,
    pub deactivated_by: String,
    pub deactivated_at: DateTime<Utc>,
}

pub fn build_party_deactivated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PartyDeactivatedPayload,
) -> EventEnvelope<PartyDeactivatedPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_PARTY_DEACTIVATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn party_created_envelope_metadata() {
        let payload = PartyCreatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "app-1".to_string(),
            party_type: "company".to_string(),
            display_name: "Acme Corp".to_string(),
            email: None,
            created_at: Utc::now(),
        };
        let env = build_party_created_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_PARTY_CREATED);
        assert_eq!(env.source_module, "party");
        assert_eq!(env.mutation_class.as_deref(), Some(MUTATION_CLASS_DATA_MUTATION));
        assert_eq!(env.schema_version, PARTY_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
    }

    #[test]
    fn party_deactivated_is_lifecycle() {
        let payload = PartyDeactivatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "app-1".to_string(),
            deactivated_by: "user-1".to_string(),
            deactivated_at: Utc::now(),
        };
        let env = build_party_deactivated_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-2".to_string(),
            None,
            payload,
        );
        assert_eq!(env.mutation_class.as_deref(), Some(MUTATION_CLASS_LIFECYCLE));
    }
}
