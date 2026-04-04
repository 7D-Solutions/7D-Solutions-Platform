//! Workflow definition CRUD — Guard→Mutation→Outbox.
//!
//! A definition is a template describing the steps and allowed transitions
//! for a class of workflows.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use super::definitions_repo::DefinitionRepo;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WorkflowDefinition {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: i32,
    pub steps: serde_json::Value,
    pub initial_step_id: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDefinitionRequest {
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: serde_json::Value,
    pub initial_step_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ListDefinitionsQuery {
    pub tenant_id: String,
    pub active_only: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum DefError {
    #[error("Definition not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Duplicate definition name+version")]
    Duplicate,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
