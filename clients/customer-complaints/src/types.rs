use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Complaints ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintDetail {
    #[serde(flatten)]
    pub complaint: Complaint,
    pub activity_log: Vec<ComplaintActivityLog>,
    pub resolution: Option<ComplaintResolution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateComplaintRequest {
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
    pub due_date: Option<DateTime<Utc>>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateComplaintRequest {
    pub customer_contact_id: Option<Uuid>,
    pub source_ref: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub source_entity_type: Option<String>,
    pub source_entity_id: Option<Uuid>,
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageComplaintRequest {
    pub category_code: String,
    pub severity: String,
    pub assigned_to: String,
    pub due_date: Option<DateTime<Utc>>,
    pub triaged_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartInvestigationRequest {
    pub started_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondComplaintRequest {
    pub responded_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseComplaintRequest {
    pub outcome: String,
    pub closed_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelComplaintRequest {
    pub reason: Option<String>,
    pub cancelled_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignComplaintRequest {
    pub assigned_to: String,
    pub assigned_by: String,
}

// ── Activity Log ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateActivityLogRequest {
    pub activity_type: Option<String>,
    pub from_value: Option<String>,
    pub to_value: Option<String>,
    pub content: Option<String>,
    pub visible_to_customer: Option<bool>,
    pub recorded_by: String,
}

// ── Resolution ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResolutionRequest {
    pub action_taken: String,
    pub root_cause_summary: Option<String>,
    pub customer_acceptance: String,
    pub customer_response_at: Option<DateTime<Utc>>,
    pub resolved_by: String,
}

// ── Category Codes ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCategoryCodeRequest {
    pub category_code: String,
    pub display_label: String,
    pub description: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateCategoryCodeRequest {
    pub display_label: Option<String>,
    pub description: Option<String>,
    pub active: Option<bool>,
    pub updated_by: String,
}

// ── Labels ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcStatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_status: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcSeverityLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_severity: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcSourceLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_source: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLabelRequest {
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
}
