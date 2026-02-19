//! Party domain types — base party, company extension, individual extension.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum PartyError {
    #[error("Party not found: {0}")]
    NotFound(Uuid),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Base Party
// ============================================================================

/// Full party base record as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Party {
    pub id: Uuid,
    pub app_id: String,
    pub party_type: String,
    pub status: String,
    pub display_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Company Extension
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PartyCompany {
    pub party_id: Uuid,
    pub legal_name: String,
    pub trade_name: Option<String>,
    pub registration_number: Option<String>,
    pub tax_id: Option<String>,
    pub country_of_incorporation: Option<String>,
    pub industry_code: Option<String>,
    pub founded_date: Option<NaiveDate>,
    pub employee_count: Option<i32>,
    pub annual_revenue_cents: Option<i64>,
    pub currency: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Individual Extension
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PartyIndividual {
    pub party_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub middle_name: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub tax_id: Option<String>,
    pub nationality: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// External Ref
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExternalRef {
    pub id: i64,
    pub party_id: Uuid,
    pub app_id: String,
    pub system: String,
    pub external_id: String,
    pub label: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Composite Responses
// ============================================================================

/// Full party view with typed extension and external refs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyView {
    #[serde(flatten)]
    pub party: Party,
    pub company: Option<PartyCompany>,
    pub individual: Option<PartyIndividual>,
    pub external_refs: Vec<ExternalRef>,
}

// ============================================================================
// Create Requests
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCompanyRequest {
    pub display_name: String,
    pub legal_name: String,
    pub trade_name: Option<String>,
    pub registration_number: Option<String>,
    pub tax_id: Option<String>,
    pub country_of_incorporation: Option<String>,
    pub industry_code: Option<String>,
    pub founded_date: Option<NaiveDate>,
    pub employee_count: Option<i32>,
    pub annual_revenue_cents: Option<i64>,
    pub currency: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateCompanyRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.display_name.trim().is_empty() {
            return Err(PartyError::Validation("display_name cannot be empty".to_string()));
        }
        if self.legal_name.trim().is_empty() {
            return Err(PartyError::Validation("legal_name cannot be empty".to_string()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIndividualRequest {
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub middle_name: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub tax_id: Option<String>,
    pub nationality: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateIndividualRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.display_name.trim().is_empty() {
            return Err(PartyError::Validation("display_name cannot be empty".to_string()));
        }
        if self.first_name.trim().is_empty() {
            return Err(PartyError::Validation("first_name cannot be empty".to_string()));
        }
        if self.last_name.trim().is_empty() {
            return Err(PartyError::Validation("last_name cannot be empty".to_string()));
        }
        Ok(())
    }
}

// ============================================================================
// Update Request (partial — applies to base party fields)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePartyRequest {
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub updated_by: Option<String>,
}

impl UpdatePartyRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref name) = self.display_name {
            if name.trim().is_empty() {
                return Err(PartyError::Validation("display_name cannot be empty".to_string()));
            }
        }
        Ok(())
    }
}

// ============================================================================
// Search
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    /// Partial match on display_name (case-insensitive)
    pub name: Option<String>,
    /// Filter by party_type: "company" or "individual"
    pub party_type: Option<String>,
    /// Filter by status (default: active)
    pub status: Option<String>,
    /// Filter by external system name (e.g. "stripe")
    pub external_system: Option<String>,
    /// Filter by external_id within the system
    pub external_id: Option<String>,
    /// Max results (default 50, max 200)
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}
