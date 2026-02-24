//! Form templates and fields domain model.
//!
//! Invariants:
//! - Templates have NO reference to any PDF file — the PDF is provided at generation time
//! - Every query filters by tenant_id for multi-tenant isolation
//! - form_fields.display_order is deterministic (no ties, contiguous integers)
//! - field_key is unique per template (DB-enforced via UNIQUE constraint)

pub mod repo;

pub use repo::{FieldRepo, TemplateRepo};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FormTemplate {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FormField {
    pub id: Uuid,
    pub template_id: Uuid,
    pub field_key: String,
    pub field_label: String,
    pub field_type: String,
    pub validation_rules: serde_json::Value,
    pub pdf_position: serde_json::Value,
    pub display_order: i32,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListTemplatesQuery {
    pub tenant_id: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFieldRequest {
    pub field_key: String,
    pub field_label: String,
    pub field_type: String,
    pub validation_rules: Option<serde_json::Value>,
    pub pdf_position: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFieldRequest {
    pub field_label: Option<String>,
    pub field_type: Option<String>,
    pub validation_rules: Option<serde_json::Value>,
    pub pdf_position: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderFieldsRequest {
    pub field_ids: Vec<Uuid>,
}

// ============================================================================
// Errors
// ============================================================================

const VALID_FIELD_TYPES: &[&str] = &["text", "number", "date", "dropdown", "checkbox"];

pub fn validate_field_type(ft: &str) -> Result<(), FormError> {
    if VALID_FIELD_TYPES.contains(&ft) {
        Ok(())
    } else {
        Err(FormError::Validation(format!(
            "invalid field_type '{}', must be one of: {}",
            ft,
            VALID_FIELD_TYPES.join(", ")
        )))
    }
}

#[derive(Debug, Error)]
pub enum FormError {
    #[error("Template not found")]
    TemplateNotFound,

    #[error("Field not found")]
    FieldNotFound,

    #[error("Duplicate field key")]
    DuplicateFieldKey,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
