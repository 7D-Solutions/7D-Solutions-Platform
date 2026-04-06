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

// ── Attachment models (DOC6) ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Attachment {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub s3_key: String,
    pub status: String,
    pub uploaded_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAttachmentRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub filename: String,
    pub mime_type: String,
    /// Declared upload size in bytes. Validated against the configured limit when present.
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AttachmentListQuery {
    pub entity_type: String,
    pub entity_id: String,
}

// ── Controlled distribution models (DOC4) ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DocumentDistribution {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub document_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: serde_json::Value,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub requested_by: Uuid,
    pub requested_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDistributionRequest {
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct DistributionStatusUpdateRequest {
    pub status: String,
    pub provider_message_id: Option<String>,
    pub failure_reason: Option<String>,
    pub notification_event_id: Option<Uuid>,
    pub delivered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentDistributionRequestedPayload {
    pub distribution_id: Uuid,
    pub document_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub doc_number: String,
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentDistributionStatusUpdatedPayload {
    pub distribution_id: Uuid,
    pub document_id: Uuid,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub failure_reason: Option<String>,
}
