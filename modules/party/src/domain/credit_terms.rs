//! Credit terms domain types — payment terms and credit limits per party.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::party::PartyError;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CreditTerms {
    pub id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub payment_terms: String,
    pub credit_limit_cents: Option<i64>,
    pub currency: String,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Create Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCreditTermsRequest {
    pub payment_terms: String,
    pub credit_limit_cents: Option<i64>,
    pub currency: Option<String>,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateCreditTermsRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.payment_terms.trim().is_empty() {
            return Err(PartyError::Validation(
                "payment_terms cannot be empty".to_string(),
            ));
        }
        if let Some(to) = self.effective_to {
            if to < self.effective_from {
                return Err(PartyError::Validation(
                    "effective_to cannot be before effective_from".to_string(),
                ));
            }
        }
        Ok(())
    }
}

// ============================================================================
// Update Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCreditTermsRequest {
    pub payment_terms: Option<String>,
    pub credit_limit_cents: Option<i64>,
    pub currency: Option<String>,
    pub effective_from: Option<NaiveDate>,
    pub effective_to: Option<NaiveDate>,
    pub notes: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl UpdateCreditTermsRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref terms) = self.payment_terms {
            if terms.trim().is_empty() {
                return Err(PartyError::Validation(
                    "payment_terms cannot be empty".to_string(),
                ));
            }
        }
        Ok(())
    }
}
