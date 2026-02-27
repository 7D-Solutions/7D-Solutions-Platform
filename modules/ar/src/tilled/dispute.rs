use super::error::TilledError;
use super::types::{Dispute, ListResponse};
use super::TilledClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct EvidenceFile {
    pub file_id: String,
    #[serde(rename = "type")]
    pub evidence_type: String,
}

#[derive(Debug, Serialize)]
pub struct SubmitEvidenceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<EvidenceFile>>,
}

impl TilledClient {
    /// Get a dispute by ID
    pub async fn get_dispute(&self, dispute_id: &str) -> Result<Dispute, TilledError> {
        let path = format!("/v1/disputes/{}", dispute_id);
        self.get(&path, None).await
    }

    /// List disputes with optional filters
    pub async fn list_disputes(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<Dispute>, TilledError> {
        self.get("/v1/disputes", filters).await
    }

    /// Submit evidence for a dispute
    pub async fn submit_dispute_evidence(
        &self,
        dispute_id: &str,
        evidence: SubmitEvidenceRequest,
    ) -> Result<Dispute, TilledError> {
        let path = format!("/v1/disputes/{}", dispute_id);
        self.post(&path, &evidence).await
    }
}

#[cfg(test)]
mod tests {
    use super::{EvidenceFile, SubmitEvidenceRequest};

    #[test]
    fn evidence_payload_serializes_file_objects() {
        let payload = SubmitEvidenceRequest {
            description: Some("REVERSAL".to_string()),
            files: Some(vec![EvidenceFile {
                file_id: "file_123".to_string(),
                evidence_type: "service_documentation".to_string(),
            }]),
        };

        let value = serde_json::to_value(payload).unwrap();
        let files = value.get("files").unwrap().as_array().unwrap();
        assert_eq!(files[0].get("file_id").unwrap(), "file_123");
        assert_eq!(files[0].get("type").unwrap(), "service_documentation");
    }
}
