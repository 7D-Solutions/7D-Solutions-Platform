//! Event contract: sync.push.failed
//!
//! Emitted when an outbound push to an external provider fails (fault or unknown).
//! Mirrors the JSON schema at contracts/events/integrations.sync.push.failed.json.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_SIDE_EFFECT};

pub const EVENT_TYPE_SYNC_PUSH_FAILED: &str = "sync.push.failed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushFailedPayload {
    pub app_id: String,
    /// OAuth connection ID through which the push was attempted.
    pub connector_id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    /// 1-based ordinal attempt number.
    pub attempt_number: u32,
    /// Human-readable explanation of why the push failed.
    pub failure_reason: String,
    /// Machine-readable failure code (e.g. rate_limited, auth_failed).
    pub failure_code: String,
    /// Whether the failure is transient and eligible for automatic retry.
    pub retryable: bool,
    /// Raw error message from the external system, if available.
    pub external_error: Option<String>,
}

pub fn build_sync_push_failed_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: SyncPushFailedPayload,
) -> EventEnvelope<SyncPushFailedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_SYNC_PUSH_FAILED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_SIDE_EFFECT.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_metadata_is_correct() {
        let payload = SyncPushFailedPayload {
            app_id: "app-1".to_string(),
            connector_id: Uuid::new_v4(),
            entity_type: "invoice".to_string(),
            entity_id: "inv-42".to_string(),
            attempt_number: 1,
            failure_reason: "QBO rejected with 400 validation error".to_string(),
            failure_code: "validation_error".to_string(),
            retryable: false,
            external_error: Some("Field 'Amount' is required".to_string()),
        };
        let env = build_sync_push_failed_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_SYNC_PUSH_FAILED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert_eq!(
            env.mutation_class.as_deref(),
            Some(MUTATION_CLASS_SIDE_EFFECT)
        );
    }

    #[test]
    fn retryable_codes_round_trip() {
        let payload = SyncPushFailedPayload {
            app_id: "app-2".to_string(),
            connector_id: Uuid::nil(),
            entity_type: "customer".to_string(),
            entity_id: "cust-1".to_string(),
            attempt_number: 2,
            failure_reason: "QBO rate limit exceeded".to_string(),
            failure_code: "rate_limited".to_string(),
            retryable: true,
            external_error: None,
        };
        let json = serde_json::to_value(&payload).expect("payload serializes");
        assert_eq!(json["retryable"], true);
        assert_eq!(json["failure_code"], "rate_limited");
        assert!(json["external_error"].is_null());
    }
}
