//! Event contract: file_job.created

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};

pub const EVENT_TYPE_FILE_JOB_CREATED: &str = "file_job.created";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileJobCreatedPayload {
    pub job_id: Uuid,
    pub tenant_id: String,
    pub file_ref: String,
    pub parser_type: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

pub fn build_file_job_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: FileJobCreatedPayload,
) -> EventEnvelope<FileJobCreatedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_FILE_JOB_CREATED.to_string(),
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
    fn file_job_created_envelope_metadata() {
        let payload = FileJobCreatedPayload {
            job_id: Uuid::new_v4(),
            tenant_id: "t-1".to_string(),
            file_ref: "s3://bucket/file.csv".to_string(),
            parser_type: "csv".to_string(),
            status: "created".to_string(),
            created_at: Utc::now(),
        };
        let env = build_file_job_created_envelope(
            Uuid::new_v4(),
            "t-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_FILE_JOB_CREATED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
    }
}
