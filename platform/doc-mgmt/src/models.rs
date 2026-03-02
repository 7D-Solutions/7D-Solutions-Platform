use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Domain model ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Document {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub doc_number: String,
    pub title: String,
    pub doc_type: String,
    pub status: String,
    pub superseded_by: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Revision {
    pub id: Uuid,
    pub document_id: Uuid,
    pub tenant_id: Uuid,
    pub revision_number: i32,
    pub body: serde_json::Value,
    pub change_summary: String,
    pub status: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

// ── API request types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateDocumentRequest {
    pub doc_number: String,
    pub title: String,
    pub doc_type: String,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRevisionRequest {
    pub body: serde_json::Value,
    pub change_summary: String,
}

// ── Event payloads ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCreatedPayload {
    pub document_id: Uuid,
    pub doc_number: String,
    pub title: String,
    pub doc_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionCreatedPayload {
    pub document_id: Uuid,
    pub revision_id: Uuid,
    pub revision_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentReleasedPayload {
    pub document_id: Uuid,
    pub doc_number: String,
    pub revision_number: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSupersededPayload {
    pub old_document_id: Uuid,
    pub new_document_id: Uuid,
    pub new_doc_number: String,
    pub old_doc_number: String,
}

#[derive(Debug, Deserialize)]
pub struct SupersedeRequest {
    pub new_doc_number: String,
    pub new_title: Option<String>,
    pub change_summary: Option<String>,
}
