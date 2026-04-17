//! Opportunity bounded context.
//!
//! Key invariants:
//! - stage_code must reference an active pipeline_stages row for the tenant
//! - advance-stage CANNOT target a terminal stage (use close-won/close-lost)
//! - close-lost requires close_reason
//! - once in terminal stage, no further stage moves are allowed

pub mod repo;
pub mod service;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum OpportunityError {
    #[error("Opportunity not found: {0}")]
    NotFound(Uuid),
    #[error("Stage '{0}' does not exist or is inactive for this tenant")]
    InvalidStage(String),
    #[error("Cannot advance to terminal stage '{0}' via advance-stage; use close-won or close-lost")]
    TerminalStageViaAdvance(String),
    #[error("Opportunity is already in terminal stage '{0}'")]
    AlreadyTerminal(String),
    #[error("close_reason is required when closing an opportunity as lost")]
    CloseLostRequiresReason,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<OpportunityError> for platform_http_contracts::ApiError {
    fn from(err: OpportunityError) -> Self {
        match err {
            OpportunityError::NotFound(id) => Self::not_found(format!("Opportunity {} not found", id)),
            OpportunityError::InvalidStage(code) => {
                Self::new(422, "invalid_stage", format!("Stage '{}' is invalid or inactive", code))
            }
            OpportunityError::TerminalStageViaAdvance(code) => Self::new(
                422,
                "terminal_stage_via_advance",
                format!("Stage '{}' is terminal; use /close-won or /close-lost", code),
            ),
            OpportunityError::AlreadyTerminal(code) => {
                Self::new(422, "already_terminal", format!("Opportunity is in terminal stage '{}'", code))
            }
            OpportunityError::CloseLostRequiresReason => {
                Self::new(422, "close_lost_requires_reason", "close_reason is required")
            }
            OpportunityError::Validation(msg) => Self::new(422, "validation_error", msg),
            OpportunityError::Database(e) => {
                tracing::error!("CRM opportunities DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ============================================================================
// Canonical Enums
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OppType {
    NewBusiness,
    RepeatOrder,
    ContractRenewal,
    EngineeringChange,
    Prototype,
}

impl OppType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OppType::NewBusiness => "new_business",
            OppType::RepeatOrder => "repeat_order",
            OppType::ContractRenewal => "contract_renewal",
            OppType::EngineeringChange => "engineering_change",
            OppType::Prototype => "prototype",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OppPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl OppPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            OppPriority::Low => "low",
            OppPriority::Medium => "medium",
            OppPriority::High => "high",
            OppPriority::Critical => "critical",
        }
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Opportunity {
    pub id: Uuid,
    pub tenant_id: String,
    pub opp_number: String,
    pub title: String,
    pub party_id: Uuid,
    pub primary_party_contact_id: Option<Uuid>,
    pub lead_id: Option<Uuid>,
    pub stage_code: String,
    pub probability_pct: i32,
    pub estimated_value_cents: Option<i64>,
    pub currency: String,
    pub expected_close_date: Option<NaiveDate>,
    pub actual_close_date: Option<NaiveDate>,
    pub close_reason: Option<String>,
    pub competitor: Option<String>,
    pub opp_type: String,
    pub priority: String,
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub external_quote_ref: Option<String>,
    pub sales_order_id: Option<Uuid>,
    pub owner_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpportunityStageHistory {
    pub id: Uuid,
    pub tenant_id: String,
    pub opportunity_id: Uuid,
    pub from_stage_code: Option<String>,
    pub to_stage_code: String,
    pub probability_pct_at_change: Option<i32>,
    pub days_in_previous_stage: Option<i32>,
    pub reason: Option<String>,
    pub notes: Option<String>,
    pub changed_by: String,
    pub changed_at: DateTime<Utc>,
}

// ============================================================================
// Request / Response Types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateOpportunityRequest {
    pub title: String,
    pub party_id: Uuid,
    pub primary_party_contact_id: Option<Uuid>,
    pub lead_id: Option<Uuid>,
    pub stage_code: Option<String>,
    pub probability_pct: Option<i32>,
    pub estimated_value_cents: Option<i64>,
    pub currency: Option<String>,
    pub expected_close_date: Option<NaiveDate>,
    pub opp_type: Option<String>,
    pub priority: Option<String>,
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub external_quote_ref: Option<String>,
    pub owner_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateOpportunityRequest {
    pub title: Option<String>,
    pub primary_party_contact_id: Option<Uuid>,
    pub probability_pct: Option<i32>,
    pub estimated_value_cents: Option<i64>,
    pub currency: Option<String>,
    pub expected_close_date: Option<NaiveDate>,
    pub opp_type: Option<String>,
    pub priority: Option<String>,
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub external_quote_ref: Option<String>,
    pub owner_id: Option<String>,
    pub competitor: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdvanceStageRequest {
    pub stage_code: String,
    pub probability_pct: Option<i32>,
    pub reason: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CloseWonRequest {
    pub sales_order_id: Option<Uuid>,
    pub reason: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CloseLostRequest {
    pub close_reason: String,
    pub competitor: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OpportunityDetail {
    pub opportunity: Opportunity,
    pub stage_history: Vec<OpportunityStageHistory>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PipelineSummaryItem {
    pub stage_code: String,
    pub display_label: String,
    pub order_rank: i32,
    pub count: i64,
    pub total_value_cents: i64,
    pub weighted_value_cents: i64,
}

#[derive(Debug, Deserialize, Default, ToSchema)]
pub struct ListOpportunitiesQuery {
    pub owner_id: Option<String>,
    pub stage_code: Option<String>,
    pub party_id: Option<Uuid>,
    pub close_date_from: Option<NaiveDate>,
    pub close_date_to: Option<NaiveDate>,
    pub include_closed: Option<bool>,
}
