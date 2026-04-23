//! Operational vitals types for the platform vitals API.
//!
//! These types are shared across SDK handlers and the control-plane aggregator.
//! All new fields must be `Option` or `Vec` to preserve forward compatibility —
//! consumers that haven't upgraded must still receive a valid response.

use serde::{Deserialize, Serialize};

/// Dead-letter queue statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqVitals {
    /// Total messages currently in the DLQ.
    pub total: u64,
    /// Messages eligible for retry.
    pub retryable: u64,
    /// Messages that failed permanently (non-retryable errors).
    pub fatal: u64,
    /// Messages that exceeded the retry limit without a clear error category.
    pub poison: u64,
}

/// Transactional outbox health.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxVitals {
    /// Number of outbox rows not yet published.
    pub pending: u64,
    /// Age of the oldest pending row in seconds, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_pending_secs: Option<u64>,
}

/// Freshness of a single read-model projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionVitals {
    /// Projection identifier (e.g. `"tenant_summary"`).
    pub name: String,
    /// Tenant this projection belongs to.
    pub tenant_id: String,
    /// Replication lag in milliseconds (event timestamp – projection cursor).
    pub lag_ms: i64,
    /// Age of the most recently applied event in seconds.
    pub age_seconds: i64,
}

/// Runtime counters for a single event consumer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerVitals {
    /// Consumer identifier (e.g. `"tenant_provisioned_consumer"`).
    pub name: String,
    /// Total messages successfully processed since last restart.
    pub processed: u64,
    /// Messages skipped (filtered out without processing).
    pub skipped: u64,
    /// Messages sent to the DLQ by this consumer.
    pub dlq: u64,
    /// Whether the consumer goroutine/task is currently running.
    pub running: bool,
}

/// Top-level vitals response returned by `GET /api/vitals`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitalsResponse {
    /// Identifying name of the reporting service (e.g. `"ar"`).
    pub service_name: String,
    /// Deployed version of the service.
    pub version: String,
    /// Whether all per-tenant setup for the calling tenant has completed.
    /// `None` when no tenant context is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_ready: Option<bool>,
    /// DLQ statistics for this service.
    pub dlq: DlqVitals,
    /// Outbox health for this service.
    pub outbox: OutboxVitals,
    /// Projection freshness snapshots.
    pub projections: Vec<ProjectionVitals>,
    /// Event consumer counters.
    pub consumers: Vec<ConsumerVitals>,
    /// Service-specific extra fields not covered by the standard schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended: Option<serde_json::Value>,
    /// RFC-3339 timestamp of when this snapshot was taken.
    pub timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_vitals() -> VitalsResponse {
        VitalsResponse {
            service_name: "ar".to_string(),
            version: "2.1.0".to_string(),
            tenant_ready: Some(true),
            dlq: DlqVitals {
                total: 3,
                retryable: 2,
                fatal: 1,
                poison: 0,
            },
            outbox: OutboxVitals {
                pending: 5,
                oldest_pending_secs: Some(42),
            },
            projections: vec![ProjectionVitals {
                name: "tenant_summary".to_string(),
                tenant_id: "00000000-0000-0000-0000-000000000001".to_string(),
                lag_ms: 120,
                age_seconds: 3,
            }],
            consumers: vec![ConsumerVitals {
                name: "tenant_provisioned_consumer".to_string(),
                processed: 1000,
                skipped: 5,
                dlq: 1,
                running: true,
            }],
            extended: None,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn vitals_response_serializes_all_fields() {
        let v = sample_vitals();
        let json = serde_json::to_value(&v).expect("serialize");

        assert_eq!(json["service_name"], "ar");
        assert_eq!(json["version"], "2.1.0");
        assert_eq!(json["tenant_ready"], true);
        assert_eq!(json["dlq"]["total"], 3);
        assert_eq!(json["dlq"]["retryable"], 2);
        assert_eq!(json["dlq"]["fatal"], 1);
        assert_eq!(json["dlq"]["poison"], 0);
        assert_eq!(json["outbox"]["pending"], 5);
        assert_eq!(json["outbox"]["oldest_pending_secs"], 42);
        assert_eq!(json["projections"][0]["name"], "tenant_summary");
        assert_eq!(json["projections"][0]["lag_ms"], 120);
        assert_eq!(json["consumers"][0]["name"], "tenant_provisioned_consumer");
        assert_eq!(json["consumers"][0]["running"], true);
        assert!(
            json["timestamp"].as_str().is_some(),
            "timestamp must be present"
        );
    }

    #[test]
    fn optional_fields_omitted_when_none() {
        let mut v = sample_vitals();
        v.tenant_ready = None;
        v.outbox.oldest_pending_secs = None;
        v.extended = None;

        let json = serde_json::to_value(&v).expect("serialize");

        assert!(
            json.get("tenant_ready").is_none(),
            "tenant_ready must be absent"
        );
        assert!(
            json["outbox"].get("oldest_pending_secs").is_none(),
            "oldest_pending_secs must be absent"
        );
        assert!(json.get("extended").is_none(), "extended must be absent");
    }

    #[test]
    fn timestamp_is_rfc3339() {
        let v = sample_vitals();
        let ts = v.timestamp.clone();
        chrono::DateTime::parse_from_rfc3339(&ts).expect("timestamp must be valid RFC-3339");
    }

    #[test]
    fn vitals_response_empty_vecs_serialize() {
        let mut v = sample_vitals();
        v.projections = vec![];
        v.consumers = vec![];

        let json = serde_json::to_value(&v).expect("serialize");
        assert_eq!(json["projections"].as_array().unwrap().len(), 0);
        assert_eq!(json["consumers"].as_array().unwrap().len(), 0);
    }
}
