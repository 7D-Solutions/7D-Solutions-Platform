use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Labels ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub canonical_value: String,
    pub display_label: String,
    pub sort_order: i32,
    pub is_active: bool,
}

// ── Pipeline Stages ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub stage_code: String,
    pub display_name: String,
    pub sort_order: i32,
    pub is_terminal: bool,
    pub is_win: bool,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStageRequest {
    pub stage_code: String,
    pub display_name: String,
    pub sort_order: i32,
    pub is_terminal: bool,
    pub is_win: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStageRequest {
    pub display_name: Option<String>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageReorderItem {
    pub stage_code: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorderStagesRequest {
    pub stages: Vec<StageReorderItem>,
}

// ── Leads ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lead {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub lead_number: String,
    pub title: String,
    pub status: String,
    pub source: Option<String>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub estimated_value: Option<f64>,
    pub currency_code: Option<String>,
    pub notes: Option<String>,
    pub disqualify_reason: Option<String>,
    pub converted_opportunity_id: Option<Uuid>,
    pub converted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLeadRequest {
    pub title: String,
    pub source: Option<String>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub estimated_value: Option<f64>,
    pub currency_code: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateLeadRequest {
    pub title: Option<String>,
    pub source: Option<String>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub estimated_value: Option<f64>,
    pub currency_code: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertLeadRequest {
    pub party_id: Uuid,
    pub opportunity_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertLeadResponse {
    pub lead_id: Uuid,
    pub opportunity_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisqualifyLeadRequest {
    pub reason: String,
}

// ── Opportunities ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub opp_number: String,
    pub title: String,
    pub stage_code: String,
    pub opp_type: Option<String>,
    pub priority: Option<String>,
    pub party_id: Option<Uuid>,
    pub primary_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub estimated_value: Option<f64>,
    pub currency_code: Option<String>,
    pub expected_close_date: Option<DateTime<Utc>>,
    pub close_reason: Option<String>,
    pub source_lead_id: Option<Uuid>,
    pub sales_order_id: Option<Uuid>,
    pub notes: Option<String>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityStageHistory {
    pub id: Uuid,
    pub opportunity_id: Uuid,
    pub stage_code: String,
    pub entered_at: DateTime<Utc>,
    pub entered_by_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityDetail {
    pub opportunity: Opportunity,
    pub stage_history: Vec<OpportunityStageHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSummaryItem {
    pub stage_code: String,
    pub display_name: String,
    pub sort_order: i32,
    pub count: i64,
    pub total_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpportunityRequest {
    pub title: String,
    pub stage_code: Option<String>,
    pub opp_type: Option<String>,
    pub priority: Option<String>,
    pub party_id: Option<Uuid>,
    pub primary_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub estimated_value: Option<f64>,
    pub currency_code: Option<String>,
    pub expected_close_date: Option<DateTime<Utc>>,
    pub source_lead_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateOpportunityRequest {
    pub title: Option<String>,
    pub opp_type: Option<String>,
    pub priority: Option<String>,
    pub party_id: Option<Uuid>,
    pub primary_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub estimated_value: Option<f64>,
    pub currency_code: Option<String>,
    pub expected_close_date: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceStageRequest {
    pub stage_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseWonRequest {
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseLostRequest {
    pub close_reason: String,
    pub notes: Option<String>,
}

// ── Activities ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub activity_type_code: String,
    pub subject: String,
    pub lead_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub due_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateActivityRequest {
    pub activity_type_code: String,
    pub subject: String,
    pub lead_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub assigned_user_id: Option<Uuid>,
    pub due_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateActivityRequest {
    pub subject: Option<String>,
    pub assigned_user_id: Option<Uuid>,
    pub due_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityType {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub type_code: String,
    pub display_name: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateActivityTypeRequest {
    pub type_code: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateActivityTypeRequest {
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
}

// ── Contact Role Attributes ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactRoleAttributes {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub party_contact_id: Uuid,
    pub sales_role: Option<String>,
    pub influence_level: Option<String>,
    pub is_decision_maker: bool,
    pub notes: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertContactRoleRequest {
    pub sales_role: Option<String>,
    pub influence_level: Option<String>,
    pub is_decision_maker: Option<bool>,
    pub notes: Option<String>,
}
