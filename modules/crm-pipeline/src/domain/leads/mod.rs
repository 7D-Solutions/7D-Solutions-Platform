//! Lead bounded context — types, state machine, validation.
//!
//! Lead lifecycle: new → contacted → qualifying → qualified → converted
//!                 (any non-terminal) → disqualified | dead
//!
//! Terminal states: converted, disqualified, dead.
//! Converted leads must have party_id set before transitioning.

pub mod repo;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum LeadError {
    #[error("Lead not found: {0}")]
    NotFound(Uuid),
    #[error("Invalid state transition from '{0}' to '{1}'")]
    InvalidTransition(String, String),
    #[error("Lead is in terminal state '{0}' — no further transitions allowed")]
    TerminalState(String),
    #[error("party_id is required to convert a lead")]
    ConversionRequiresParty,
    #[error("disqualify_reason is required to disqualify a lead")]
    DisqualifyRequiresReason,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("party auto-create failed: {0}")]
    PartyApiError(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<LeadError> for platform_http_contracts::ApiError {
    fn from(err: LeadError) -> Self {
        match err {
            LeadError::NotFound(id) => Self::not_found(format!("Lead {} not found", id)),
            LeadError::InvalidTransition(from, to) => Self::new(
                422,
                "invalid_transition",
                format!("Cannot move lead from '{}' to '{}'", from, to),
            ),
            LeadError::TerminalState(s) => Self::new(
                422,
                "terminal_state",
                format!("Lead is in terminal state '{}'", s),
            ),
            LeadError::ConversionRequiresParty => Self::new(
                422,
                "conversion_requires_party",
                "party_id must be set before converting a lead",
            ),
            LeadError::DisqualifyRequiresReason => Self::new(
                422,
                "disqualify_requires_reason",
                "disqualify_reason is required",
            ),
            LeadError::Validation(msg) => Self::new(422, "validation_error", msg),
            LeadError::PartyApiError(msg) => Self::new(502, "party_api_error", msg),
            LeadError::Database(e) => {
                tracing::error!("CRM leads DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ============================================================================
// Lead Status State Machine
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum LeadStatus {
    New,
    Contacted,
    Qualifying,
    Qualified,
    Converted,
    Disqualified,
    Dead,
}

impl LeadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LeadStatus::New => "new",
            LeadStatus::Contacted => "contacted",
            LeadStatus::Qualifying => "qualifying",
            LeadStatus::Qualified => "qualified",
            LeadStatus::Converted => "converted",
            LeadStatus::Disqualified => "disqualified",
            LeadStatus::Dead => "dead",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "new" => Some(LeadStatus::New),
            "contacted" => Some(LeadStatus::Contacted),
            "qualifying" => Some(LeadStatus::Qualifying),
            "qualified" => Some(LeadStatus::Qualified),
            "converted" => Some(LeadStatus::Converted),
            "disqualified" => Some(LeadStatus::Disqualified),
            "dead" => Some(LeadStatus::Dead),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            LeadStatus::Converted | LeadStatus::Disqualified | LeadStatus::Dead
        )
    }

    /// Returns true if this status transition is valid per the state machine.
    pub fn can_transition_to(&self, next: &LeadStatus) -> bool {
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // Forward progression
            (LeadStatus::New, LeadStatus::Contacted) => true,
            (LeadStatus::Contacted, LeadStatus::Qualifying) => true,
            (LeadStatus::Qualifying, LeadStatus::Qualified) => true,
            (LeadStatus::Qualified, LeadStatus::Converted) => true,
            // Any non-terminal → dead or disqualified
            (_, LeadStatus::Disqualified) => true,
            (_, LeadStatus::Dead) => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for LeadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Lead Source (canonical)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LeadSource {
    Website,
    TradeShow,
    Referral,
    Rfq,
    ColdCall,
    ExistingCustomer,
    Other,
}

impl LeadSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            LeadSource::Website => "website",
            LeadSource::TradeShow => "trade_show",
            LeadSource::Referral => "referral",
            LeadSource::Rfq => "rfq",
            LeadSource::ColdCall => "cold_call",
            LeadSource::ExistingCustomer => "existing_customer",
            LeadSource::Other => "other",
        }
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Lead {
    pub id: Uuid,
    pub tenant_id: String,
    pub lead_number: String,
    pub source: String,
    pub source_detail: Option<String>,
    pub company_name: String,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_title: Option<String>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub status: String,
    pub disqualify_reason: Option<String>,
    pub estimated_value_cents: Option<i64>,
    pub currency: String,
    pub converted_opportunity_id: Option<Uuid>,
    pub converted_at: Option<DateTime<Utc>>,
    pub owner_id: Option<String>,
    pub notes: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request Types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLeadRequest {
    pub source: String,
    pub source_detail: Option<String>,
    pub company_name: String,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_title: Option<String>,
    pub estimated_value_cents: Option<i64>,
    pub currency: Option<String>,
    pub owner_id: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLeadRequest {
    pub source: Option<String>,
    pub source_detail: Option<String>,
    pub company_name: Option<String>,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_title: Option<String>,
    pub estimated_value_cents: Option<i64>,
    pub currency: Option<String>,
    pub owner_id: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConvertLeadRequest {
    /// Pre-existing Party ID to link, or None to indicate Party must be created externally first.
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub opportunity_title: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DisqualifyLeadRequest {
    pub reason: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConvertLeadResponse {
    pub lead: Lead,
    pub opportunity_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, Default, ToSchema)]
pub struct ListLeadsQuery {
    pub status: Option<String>,
    pub owner_id: Option<String>,
    pub include_terminal: Option<bool>,
}
