//! Contact role domain types — structured role assignments per contact per party.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::party::PartyError;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ContactRole {
    pub id: Uuid,
    pub party_id: Uuid,
    pub contact_id: Uuid,
    pub app_id: String,
    pub role_type: String,
    pub is_primary: bool,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Create Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContactRoleRequest {
    pub contact_id: Uuid,
    pub role_type: String,
    pub is_primary: Option<bool>,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateContactRoleRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.role_type.trim().is_empty() {
            return Err(PartyError::Validation(
                "role_type cannot be empty".to_string(),
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
pub struct UpdateContactRoleRequest {
    pub role_type: Option<String>,
    pub is_primary: Option<bool>,
    pub effective_from: Option<NaiveDate>,
    pub effective_to: Option<NaiveDate>,
    pub metadata: Option<serde_json::Value>,
}

impl UpdateContactRoleRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref rt) = self.role_type {
            if rt.trim().is_empty() {
                return Err(PartyError::Validation(
                    "role_type cannot be empty".to_string(),
                ));
            }
        }
        Ok(())
    }
}
