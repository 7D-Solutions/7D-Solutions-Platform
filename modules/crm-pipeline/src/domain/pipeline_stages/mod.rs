//! Pipeline stage definitions — tenant-configurable, ordered.
//!
//! Invariant: each tenant must have exactly one initial stage
//! (lowest order_rank among active non-terminal stages).
//! Terminal stages require explicit close endpoints; generic advance-stage
//! refuses to target them.

pub mod repo;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Default seed stages
// ============================================================================

pub const DEFAULT_STAGES: &[(&str, &str, i32, bool, bool, Option<i32>)] = &[
    ("prospecting",        "Prospecting",         10, false, false, Some(10)),
    ("discovery",          "Discovery",           20, false, false, Some(25)),
    ("proposal",           "Proposal",            30, false, false, Some(40)),
    ("negotiation",        "Negotiation",         40, false, false, Some(60)),
    ("awaiting_commitment","Awaiting Commitment",  50, false, false, Some(80)),
    ("closed_won",         "Closed Won",          60, true,  true,  Some(100)),
    ("closed_lost",        "Closed Lost",         70, true,  false, Some(0)),
];

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum StageError {
    #[error("Stage not found: {0}")]
    NotFound(String),
    #[error("Duplicate stage code '{0}' already exists for tenant")]
    DuplicateCode(String),
    #[error("Tenant pipeline must have exactly one initial (lowest order_rank non-terminal active) stage")]
    MultipleInitialStages,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<StageError> for platform_http_contracts::ApiError {
    fn from(err: StageError) -> Self {
        match err {
            StageError::NotFound(code) => Self::not_found(format!("Stage '{}' not found", code)),
            StageError::DuplicateCode(code) => {
                Self::conflict(format!("Stage code '{}' already exists", code))
            }
            StageError::MultipleInitialStages => {
                Self::new(422, "multiple_initial_stages", "Pipeline must have exactly one initial stage")
            }
            StageError::Validation(msg) => Self::new(422, "validation_error", msg),
            StageError::Database(e) => {
                tracing::error!("CRM stages DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct PipelineStage {
    pub id: Uuid,
    pub tenant_id: String,
    pub stage_code: String,
    pub display_label: String,
    pub description: Option<String>,
    pub order_rank: i32,
    pub is_terminal: bool,
    pub is_win: bool,
    pub probability_default_pct: Option<i32>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

// ============================================================================
// Request Types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateStageRequest {
    pub stage_code: String,
    pub display_label: String,
    pub description: Option<String>,
    pub order_rank: i32,
    pub is_terminal: bool,
    pub is_win: Option<bool>,
    pub probability_default_pct: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateStageRequest {
    pub display_label: Option<String>,
    pub description: Option<String>,
    pub order_rank: Option<i32>,
    pub probability_default_pct: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReorderStagesRequest {
    pub stages: Vec<StageReorderItem>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StageReorderItem {
    pub stage_code: String,
    pub order_rank: i32,
}
