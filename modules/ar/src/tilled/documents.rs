use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Document response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct Document {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "type")]
    pub document_type: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Body for submitting a single document.
#[derive(Debug, Clone, Serialize)]
pub struct SubmitDocumentBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
}

/// Body for bulk-submitting documents.
#[derive(Debug, Clone, Serialize)]
pub struct BulkSubmitDocumentsBody {
    pub document_ids: Vec<String>,
}

impl TilledClient {
    /// List documents with optional filters.
    pub async fn list_documents(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<Document>, TilledError> {
        self.get("/v1/documents", filters).await
    }

    /// Get a document by ID.
    pub async fn get_document(&self, id: &str) -> Result<Document, TilledError> {
        let path = format!("/v1/documents/{id}");
        self.get(&path, None).await
    }

    /// Submit a document by ID.
    pub async fn submit_document(
        &self,
        id: &str,
        body: &SubmitDocumentBody,
    ) -> Result<Document, TilledError> {
        let path = format!("/v1/documents/{id}/submit");
        self.post(&path, body).await
    }

    /// Bulk submit documents by their IDs.
    pub async fn bulk_submit_documents(
        &self,
        body: &BulkSubmitDocumentsBody,
    ) -> Result<Vec<Document>, TilledError> {
        self.post("/v1/documents/bulk-submit", body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_deserializes_full() {
        let value = serde_json::json!({
            "id": "doc_123",
            "status": "pending",
            "type": "identity_verification",
            "account_id": "acct_456",
            "file_id": "file_789",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let doc: Document = serde_json::from_value(value).unwrap();
        assert_eq!(doc.id, "doc_123");
        assert_eq!(doc.status.as_deref(), Some("pending"));
        assert_eq!(doc.document_type.as_deref(), Some("identity_verification"));
    }

    #[test]
    fn document_deserializes_minimal() {
        let value = serde_json::json!({"id": "doc_min"});
        let doc: Document = serde_json::from_value(value).unwrap();
        assert_eq!(doc.id, "doc_min");
        assert!(doc.status.is_none());
    }

    #[test]
    fn submit_body_serializes_with_file() {
        let body = SubmitDocumentBody {
            file_id: Some("file_abc".to_string()),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["file_id"], "file_abc");
    }

    #[test]
    fn submit_body_serializes_without_file() {
        let body = SubmitDocumentBody { file_id: None };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("file_id").is_none());
    }

    #[test]
    fn bulk_submit_body_serializes() {
        let body = BulkSubmitDocumentsBody {
            document_ids: vec!["doc_1".to_string(), "doc_2".to_string()],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["document_ids"].as_array().unwrap().len(), 2);
    }
}
