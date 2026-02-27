use super::error::TilledError;
use super::types::{Dispute, ListResponse};
use super::TilledClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct SubmitEvidenceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_communication: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncategorized_text: Option<String>,
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
        let path = format!("/v1/disputes/{}/evidence", dispute_id);
        self.post(&path, &evidence).await
    }
}
