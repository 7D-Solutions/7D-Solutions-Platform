//! Training delivery domain: plans, assignments, completions.
//!
//! Invariant: a completion with outcome=passed MUST atomically create a
//! competence_assignment in the same DB transaction. See core::record_training_completion.

pub mod core;
pub mod queries;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub use core::{
    create_training_assignment, create_training_plan, record_training_completion,
    transition_assignment_status,
};
pub use queries::{
    get_training_assignment, get_training_plan, list_training_assignments, list_training_completions,
    list_training_plans,
};

// ============================================================================
// Enums
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TrainingStatus {
    Assigned,
    Scheduled,
    InProgress,
    Completed,
    Cancelled,
    NoShow,
}

impl std::fmt::Display for TrainingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrainingStatus::Assigned => write!(f, "assigned"),
            TrainingStatus::Scheduled => write!(f, "scheduled"),
            TrainingStatus::InProgress => write!(f, "in_progress"),
            TrainingStatus::Completed => write!(f, "completed"),
            TrainingStatus::Cancelled => write!(f, "cancelled"),
            TrainingStatus::NoShow => write!(f, "no_show"),
        }
    }
}

impl std::str::FromStr for TrainingStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "assigned" => Ok(TrainingStatus::Assigned),
            "scheduled" => Ok(TrainingStatus::Scheduled),
            "in_progress" => Ok(TrainingStatus::InProgress),
            "completed" => Ok(TrainingStatus::Completed),
            "cancelled" => Ok(TrainingStatus::Cancelled),
            "no_show" => Ok(TrainingStatus::NoShow),
            _ => Err(format!("unknown training status: '{s}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TrainingOutcome {
    Passed,
    Failed,
    Incomplete,
}

impl std::fmt::Display for TrainingOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrainingOutcome::Passed => write!(f, "passed"),
            TrainingOutcome::Failed => write!(f, "failed"),
            TrainingOutcome::Incomplete => write!(f, "incomplete"),
        }
    }
}

impl std::str::FromStr for TrainingOutcome {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "passed" => Ok(TrainingOutcome::Passed),
            "failed" => Ok(TrainingOutcome::Failed),
            "incomplete" => Ok(TrainingOutcome::Incomplete),
            _ => Err(format!("unknown training outcome: '{s}'")),
        }
    }
}

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrainingPlan {
    pub id: Uuid,
    pub tenant_id: String,
    pub plan_code: String,
    pub title: String,
    pub description: Option<String>,
    pub artifact_id: Uuid,
    pub duration_minutes: i32,
    pub instructor_id: Option<Uuid>,
    pub material_refs: Vec<String>,
    pub required_for_artifact_codes: Vec<String>,
    pub location: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrainingAssignment {
    pub id: Uuid,
    pub tenant_id: String,
    pub plan_id: Uuid,
    pub operator_id: Uuid,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
    pub status: TrainingStatus,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrainingCompletion {
    pub id: Uuid,
    pub tenant_id: String,
    pub assignment_id: Uuid,
    pub operator_id: Uuid,
    pub plan_id: Uuid,
    pub completed_at: DateTime<Utc>,
    pub verified_by: Option<String>,
    pub outcome: TrainingOutcome,
    pub notes: Option<String>,
    pub resulting_competence_assignment_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Requests
// ============================================================================

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTrainingPlanRequest {
    pub tenant_id: String,
    pub plan_code: String,
    pub title: String,
    pub description: Option<String>,
    pub artifact_id: Uuid,
    pub duration_minutes: i32,
    pub instructor_id: Option<Uuid>,
    pub material_refs: Option<Vec<String>>,
    pub required_for_artifact_codes: Option<Vec<String>>,
    pub location: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub updated_by: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTrainingAssignmentRequest {
    pub tenant_id: String,
    pub plan_id: Uuid,
    pub operator_id: Uuid,
    pub assigned_by: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransitionAssignmentRequest {
    pub tenant_id: String,
    pub assignment_id: Uuid,
    pub new_status: TrainingStatus,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RecordCompletionRequest {
    pub tenant_id: String,
    pub assignment_id: Uuid,
    pub completed_at: DateTime<Utc>,
    pub verified_by: Option<String>,
    pub outcome: TrainingOutcome,
    pub notes: Option<String>,
    /// Evidence reference stored on the auto-created competence_assignment when outcome=passed.
    pub evidence_ref: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

// ============================================================================
// Internal DB row types (shared between core and queries)
// ============================================================================

#[derive(sqlx::FromRow)]
pub(super) struct TrainingPlanRow {
    pub(super) id: Uuid,
    pub(super) tenant_id: String,
    pub(super) plan_code: String,
    pub(super) title: String,
    pub(super) description: Option<String>,
    pub(super) artifact_id: Uuid,
    pub(super) duration_minutes: i32,
    pub(super) instructor_id: Option<Uuid>,
    pub(super) material_refs: Vec<String>,
    pub(super) required_for_artifact_codes: Vec<String>,
    pub(super) location: Option<String>,
    pub(super) scheduled_at: Option<DateTime<Utc>>,
    pub(super) active: bool,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
    pub(super) updated_by: Option<String>,
}

impl From<TrainingPlanRow> for TrainingPlan {
    fn from(r: TrainingPlanRow) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id,
            plan_code: r.plan_code,
            title: r.title,
            description: r.description,
            artifact_id: r.artifact_id,
            duration_minutes: r.duration_minutes,
            instructor_id: r.instructor_id,
            material_refs: r.material_refs,
            required_for_artifact_codes: r.required_for_artifact_codes,
            location: r.location,
            scheduled_at: r.scheduled_at,
            active: r.active,
            created_at: r.created_at,
            updated_at: r.updated_at,
            updated_by: r.updated_by,
        }
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct TrainingAssignmentRow {
    pub(super) id: Uuid,
    pub(super) tenant_id: String,
    pub(super) plan_id: Uuid,
    pub(super) operator_id: Uuid,
    pub(super) assigned_by: String,
    pub(super) assigned_at: DateTime<Utc>,
    pub(super) status: String,
    pub(super) scheduled_at: Option<DateTime<Utc>>,
    pub(super) notes: Option<String>,
    pub(super) updated_at: DateTime<Utc>,
}

impl From<TrainingAssignmentRow> for TrainingAssignment {
    fn from(r: TrainingAssignmentRow) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id,
            plan_id: r.plan_id,
            operator_id: r.operator_id,
            assigned_by: r.assigned_by,
            assigned_at: r.assigned_at,
            status: r.status.parse().unwrap_or(TrainingStatus::Assigned),
            scheduled_at: r.scheduled_at,
            notes: r.notes,
            updated_at: r.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct TrainingCompletionRow {
    pub(super) id: Uuid,
    pub(super) tenant_id: String,
    pub(super) assignment_id: Uuid,
    pub(super) operator_id: Uuid,
    pub(super) plan_id: Uuid,
    pub(super) completed_at: DateTime<Utc>,
    pub(super) verified_by: Option<String>,
    pub(super) outcome: String,
    pub(super) notes: Option<String>,
    pub(super) resulting_competence_assignment_id: Option<Uuid>,
    pub(super) created_at: DateTime<Utc>,
}

impl From<TrainingCompletionRow> for TrainingCompletion {
    fn from(r: TrainingCompletionRow) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id,
            assignment_id: r.assignment_id,
            operator_id: r.operator_id,
            plan_id: r.plan_id,
            completed_at: r.completed_at,
            verified_by: r.verified_by,
            outcome: r.outcome.parse().unwrap_or(TrainingOutcome::Incomplete),
            notes: r.notes,
            resulting_competence_assignment_id: r.resulting_competence_assignment_id,
            created_at: r.created_at,
        }
    }
}

// ============================================================================
// Event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct TrainingPlannedPayload {
    pub plan_id: Uuid,
    pub tenant_id: String,
    pub plan_code: String,
    pub artifact_id: Uuid,
    pub title: String,
    pub scheduled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrainingAssignedPayload {
    pub assignment_id: Uuid,
    pub tenant_id: String,
    pub plan_id: Uuid,
    pub operator_id: Uuid,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrainingCompletedPayload {
    pub completion_id: Uuid,
    pub tenant_id: String,
    pub assignment_id: Uuid,
    pub operator_id: Uuid,
    pub plan_id: Uuid,
    pub completed_at: DateTime<Utc>,
    pub outcome: String,
    pub resulting_competence_assignment_id: Option<Uuid>,
}
