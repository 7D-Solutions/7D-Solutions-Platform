use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Status enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStatus {
    Draft,
    Released,
}

impl RoutingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Released => "released",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "released" => Some(Self::Released),
            _ => None,
        }
    }
}

// ============================================================================
// Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct RoutingTemplate {
    pub routing_template_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub item_id: Option<Uuid>,
    pub bom_revision_id: Option<Uuid>,
    pub revision: String,
    pub status: String,
    pub effective_from_date: Option<NaiveDate>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct RoutingStep {
    pub routing_step_id: Uuid,
    pub routing_template_id: Uuid,
    pub sequence_number: i32,
    pub workcenter_id: Uuid,
    pub operation_name: String,
    pub description: Option<String>,
    pub setup_time_minutes: Option<i32>,
    pub run_time_minutes: Option<i32>,
    pub is_required: bool,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateRoutingRequest {
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub item_id: Option<Uuid>,
    pub bom_revision_id: Option<Uuid>,
    pub revision: Option<String>,
    pub effective_from_date: Option<NaiveDate>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRoutingRequest {
    pub tenant_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub effective_from_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddRoutingStepRequest {
    pub tenant_id: String,
    pub sequence_number: i32,
    pub workcenter_id: Uuid,
    pub operation_name: String,
    pub description: Option<String>,
    pub setup_time_minutes: Option<i32>,
    pub run_time_minutes: Option<i32>,
    pub is_required: Option<bool>,
    pub idempotency_key: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("Routing not found")]
    NotFound,

    #[error("Duplicate revision '{0}' for item in tenant '{1}'")]
    DuplicateRevision(String, String),

    #[error("Duplicate sequence number {0} for routing")]
    DuplicateSequence(i32),

    #[error("Workcenter '{0}' not found or inactive")]
    WorkcenterInvalid(Uuid),

    #[error("Invalid state transition: cannot move from '{from}' to '{to}'")]
    InvalidTransition { from: String, to: String },

    #[error("Cannot modify released routing")]
    ReleasedImmutable,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflicting idempotency key")]
    ConflictingIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
