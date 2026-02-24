//! Form submissions domain model.
//!
//! Invariants:
//! - Submissions start as `draft`, transition to `submitted` (no regression)
//! - Autosave updates field_data only while in `draft` status
//! - Submit validates field_data against template field definitions
//! - Every query filters by tenant_id for multi-tenant isolation
//! - Emits pdf.form.submitted event on successful submission

pub mod repo;
pub mod validation;

pub use repo::SubmissionRepo;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FormSubmission {
    pub id: Uuid,
    pub tenant_id: String,
    pub template_id: Uuid,
    pub submitted_by: String,
    pub status: String,
    pub field_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateSubmissionRequest {
    pub tenant_id: String,
    pub template_id: Uuid,
    pub submitted_by: String,
    pub field_data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AutosaveRequest {
    pub field_data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct ListSubmissionsQuery {
    pub tenant_id: String,
    pub template_id: Option<Uuid>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// Event payload
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct FormSubmittedPayload {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub template_id: Uuid,
    pub submitted_by: String,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum SubmissionError {
    #[error("Submission not found")]
    NotFound,

    #[error("Template not found")]
    TemplateNotFound,

    #[error("Already submitted")]
    AlreadySubmitted,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
