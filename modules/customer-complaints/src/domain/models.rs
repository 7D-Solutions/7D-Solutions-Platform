use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ComplaintError {
    #[error("Complaint not found: {0}")]
    NotFound(Uuid),

    #[error("Invalid state transition from '{from}' to '{to}': {reason}")]
    InvalidTransition {
        from: String,
        to: String,
        reason: String,
    },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Append-only: {0} records cannot be modified or deleted")]
    AppendOnly(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<ComplaintError> for platform_http_contracts::ApiError {
    fn from(err: ComplaintError) -> Self {
        match err {
            ComplaintError::NotFound(id) => Self::not_found(format!("Complaint {} not found", id)),
            ComplaintError::InvalidTransition { from, to, reason } => Self::new(
                422,
                "invalid_transition",
                format!("Cannot transition from '{}' to '{}': {}", from, to, reason),
            ),
            ComplaintError::Validation(msg) => Self::new(422, "validation_error", msg),
            ComplaintError::Conflict(msg) => Self::new(409, "conflict", msg),
            ComplaintError::AppendOnly(entity) => Self::new(
                422,
                "append_only",
                format!("{} records cannot be modified or deleted", entity),
            ),
            ComplaintError::Database(e) => {
                tracing::error!("CC database error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ── Status ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComplaintStatus {
    Intake,
    Triaged,
    Investigating,
    Responded,
    Closed,
    Cancelled,
}

impl ComplaintStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Intake => "intake",
            Self::Triaged => "triaged",
            Self::Investigating => "investigating",
            Self::Responded => "responded",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "intake" => Some(Self::Intake),
            "triaged" => Some(Self::Triaged),
            "investigating" => Some(Self::Investigating),
            "responded" => Some(Self::Responded),
            "closed" => Some(Self::Closed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }
}

// ── Severity (canonical) ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComplaintSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl ComplaintSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

// ── Source (canonical) ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComplaintSource {
    Phone,
    Email,
    Portal,
    Survey,
    ServiceTicket,
    WalkIn,
    Letter,
    Other,
}

impl ComplaintSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Email => "email",
            Self::Portal => "portal",
            Self::Survey => "survey",
            Self::ServiceTicket => "service_ticket",
            Self::WalkIn => "walk_in",
            Self::Letter => "letter",
            Self::Other => "other",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "phone" => Some(Self::Phone),
            "email" => Some(Self::Email),
            "portal" => Some(Self::Portal),
            "survey" => Some(Self::Survey),
            "service_ticket" => Some(Self::ServiceTicket),
            "walk_in" => Some(Self::WalkIn),
            "letter" => Some(Self::Letter),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

// ── Outcome (canonical) ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComplaintOutcome {
    Resolved,
    Unresolvable,
    CustomerWithdrew,
    Duplicate,
}

impl ComplaintOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Unresolvable => "unresolvable",
            Self::CustomerWithdrew => "customer_withdrew",
            Self::Duplicate => "duplicate",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "resolved" => Some(Self::Resolved),
            "unresolvable" => Some(Self::Unresolvable),
            "customer_withdrew" => Some(Self::CustomerWithdrew),
            "duplicate" => Some(Self::Duplicate),
            _ => None,
        }
    }
}

// ── Customer Acceptance (canonical) ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CustomerAcceptance {
    Accepted,
    Rejected,
    NoResponse,
    NotApplicable,
}

impl CustomerAcceptance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::NoResponse => "no_response",
            Self::NotApplicable => "n_a",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            "no_response" => Some(Self::NoResponse),
            "n_a" => Some(Self::NotApplicable),
            _ => None,
        }
    }
}

// ── Activity Type (canonical) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    StatusChange,
    Note,
    CustomerCommunication,
    InternalCommunication,
    AttachmentAdded,
    AssignmentChange,
}

impl ActivityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StatusChange => "status_change",
            Self::Note => "note",
            Self::CustomerCommunication => "customer_communication",
            Self::InternalCommunication => "internal_communication",
            Self::AttachmentAdded => "attachment_added",
            Self::AssignmentChange => "assignment_change",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "status_change" => Some(Self::StatusChange),
            "note" => Some(Self::Note),
            "customer_communication" => Some(Self::CustomerCommunication),
            "internal_communication" => Some(Self::InternalCommunication),
            "attachment_added" => Some(Self::AttachmentAdded),
            "assignment_change" => Some(Self::AssignmentChange),
            _ => None,
        }
    }
}

// ── Complaint ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Complaint {
    pub id: Uuid,
    pub tenant_id: String,
    pub complaint_number: String,
    pub status: String,
    pub party_id: Uuid,
    pub customer_contact_id: Option<Uuid>,
    pub source: String,
    pub source_ref: Option<String>,
    pub severity: Option<String>,
    pub category_code: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub source_entity_type: Option<String>,
    pub source_entity_id: Option<Uuid>,
    pub assigned_to: Option<String>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub due_date: Option<DateTime<Utc>>,
    pub overdue_emitted_at: Option<DateTime<Utc>>,
    pub received_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub responded_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub outcome: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComplaintDetail {
    #[serde(flatten)]
    pub complaint: Complaint,
    pub activity_log: Vec<ComplaintActivityLog>,
    pub resolution: Option<ComplaintResolution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateComplaintRequest {
    pub party_id: Uuid,
    pub customer_contact_id: Option<Uuid>,
    pub source: ComplaintSource,
    pub source_ref: Option<String>,
    pub severity: Option<ComplaintSeverity>,
    pub category_code: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub source_entity_type: Option<String>,
    pub source_entity_id: Option<Uuid>,
    pub due_date: Option<DateTime<Utc>>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateComplaintRequest {
    pub customer_contact_id: Option<Uuid>,
    pub source_ref: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub source_entity_type: Option<String>,
    pub source_entity_id: Option<Uuid>,
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TriageComplaintRequest {
    pub category_code: String,
    pub severity: ComplaintSeverity,
    pub assigned_to: String,
    pub due_date: Option<DateTime<Utc>>,
    pub triaged_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartInvestigationRequest {
    pub started_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RespondComplaintRequest {
    pub responded_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CloseComplaintRequest {
    pub outcome: ComplaintOutcome,
    pub closed_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelComplaintRequest {
    pub reason: Option<String>,
    pub cancelled_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssignComplaintRequest {
    pub assigned_to: String,
    pub assigned_by: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListComplaintsQuery {
    pub status: Option<String>,
    pub severity: Option<String>,
    pub category_code: Option<String>,
    pub party_id: Option<Uuid>,
    pub assigned_to: Option<String>,
    pub source_entity_type: Option<String>,
    pub from_date: Option<DateTime<Utc>>,
    pub to_date: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Complaint Activity Log ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ComplaintActivityLog {
    pub id: Uuid,
    pub tenant_id: String,
    pub complaint_id: Uuid,
    pub activity_type: String,
    pub from_value: Option<String>,
    pub to_value: Option<String>,
    pub content: Option<String>,
    pub visible_to_customer: bool,
    pub recorded_by: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateActivityLogRequest {
    pub activity_type: ActivityType,
    pub from_value: Option<String>,
    pub to_value: Option<String>,
    pub content: Option<String>,
    pub visible_to_customer: Option<bool>,
    pub recorded_by: String,
}

// ── Complaint Resolution ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ComplaintResolution {
    pub id: Uuid,
    pub tenant_id: String,
    pub complaint_id: Uuid,
    pub action_taken: String,
    pub root_cause_summary: Option<String>,
    pub customer_acceptance: String,
    pub customer_response_at: Option<DateTime<Utc>>,
    pub resolved_by: String,
    pub resolved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateResolutionRequest {
    pub action_taken: String,
    pub root_cause_summary: Option<String>,
    pub customer_acceptance: CustomerAcceptance,
    pub customer_response_at: Option<DateTime<Utc>>,
    pub resolved_by: String,
}

// ── Category Codes ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ComplaintCategoryCode {
    pub id: Uuid,
    pub tenant_id: String,
    pub category_code: String,
    pub display_label: String,
    pub description: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCategoryCodeRequest {
    pub category_code: String,
    pub display_label: String,
    pub description: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateCategoryCodeRequest {
    pub display_label: Option<String>,
    pub description: Option<String>,
    pub active: Option<bool>,
    pub updated_by: String,
}

// ── Label Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CcStatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_status: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CcSeverityLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_severity: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CcSourceLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_source: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertLabelRequest {
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
}
