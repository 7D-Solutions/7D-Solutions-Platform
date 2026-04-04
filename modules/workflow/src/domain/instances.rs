//! Workflow instance lifecycle — Guard→Mutation→Outbox.
//!
//! Invariants:
//! - Every advance is validated against the definition's step list.
//! - Every transition is recorded in workflow_transitions for full audit.
//! - Idempotency: duplicate advance keys return the existing transition without re-executing.
//! - All mutations + outbox writes happen in a single transaction.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::types::InstanceStatus;

pub use super::instances_repo::InstanceRepo;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WorkflowInstance {
    pub id: Uuid,
    pub tenant_id: String,
    pub definition_id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub current_step_id: String,
    #[sqlx(try_from = "String")]
    pub status: InstanceStatus,
    pub context: serde_json::Value,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct WorkflowTransition {
    pub id: Uuid,
    pub tenant_id: String,
    pub instance_id: Uuid,
    pub from_step_id: String,
    pub to_step_id: String,
    pub action: String,
    pub actor_id: Option<Uuid>,
    pub actor_type: Option<String>,
    pub comment: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
    pub transitioned_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct StartInstanceRequest {
    pub tenant_id: String,
    pub definition_id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub context: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AdvanceInstanceRequest {
    pub tenant_id: String,
    pub to_step_id: String,
    pub action: String,
    pub actor_id: Option<Uuid>,
    pub actor_type: Option<String>,
    pub comment: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListInstancesQuery {
    pub tenant_id: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub status: Option<String>,
    pub definition_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum InstanceError {
    #[error("Instance not found")]
    NotFound,

    #[error("Definition not found")]
    DefinitionNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid transition: {0}")]
    InvalidTransition(String),

    #[error("Instance is not active (status: {0})")]
    NotActive(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
