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

// ── Retention + legal hold models ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RetentionPolicy {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub doc_type: String,
    pub retention_days: i32,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LegalHold {
    pub id: Uuid,
    pub document_id: Uuid,
    pub tenant_id: Uuid,
    pub reason: String,
    pub held_by: Uuid,
    pub held_at: DateTime<Utc>,
    pub released_by: Option<Uuid>,
    pub released_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct SetRetentionPolicyRequest {
    pub doc_type: String,
    pub retention_days: i32,
}

#[derive(Debug, Deserialize)]
pub struct ApplyHoldRequest {
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseHoldRequest {
    pub reason: String,
}

// ── Retention + hold event payloads ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentDisposedPayload {
    pub document_id: Uuid,
    pub doc_number: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalHoldAppliedPayload {
    pub document_id: Uuid,
    pub hold_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalHoldReleasedPayload {
    pub document_id: Uuid,
    pub hold_id: Uuid,
    pub reason: String,
}

// ── Template engine models ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DocTemplate {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub doc_type: String,
    pub body_template: serde_json::Value,
    pub version: i32,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RenderArtifact {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub template_id: Uuid,
    pub idempotency_key: Option<String>,
    pub input_hash: String,
    pub output_hash: String,
    pub output: serde_json::Value,
    pub rendered_by: Uuid,
    pub rendered_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub doc_type: String,
    pub body_template: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct RenderRequest {
    pub input_data: serde_json::Value,
}

// ── Template + render event payloads ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateCreatedPayload {
    pub template_id: Uuid,
    pub name: String,
    pub doc_type: String,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRenderedPayload {
    pub artifact_id: Uuid,
    pub template_id: Uuid,
    pub output_hash: String,
}
