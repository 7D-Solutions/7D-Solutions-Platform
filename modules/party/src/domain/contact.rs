//! Contact domain types — a named person linked to a party.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::party::PartyError;

// ============================================================================
// Contact Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Contact {
    pub id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub role: Option<String>,
    pub is_primary: bool,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Create Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContactRequest {
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub role: Option<String>,
    pub is_primary: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateContactRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.first_name.trim().is_empty() {
            return Err(PartyError::Validation(
                "first_name cannot be empty".to_string(),
            ));
        }
        if self.last_name.trim().is_empty() {
            return Err(PartyError::Validation(
                "last_name cannot be empty".to_string(),
            ));
        }
        if let Some(ref email) = self.email {
            if !email.contains('@') || email.trim().is_empty() {
                return Err(PartyError::Validation(
                    "email must be a valid email address".to_string(),
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
pub struct UpdateContactRequest {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub role: Option<String>,
    pub is_primary: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

impl UpdateContactRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref name) = self.first_name {
            if name.trim().is_empty() {
                return Err(PartyError::Validation(
                    "first_name cannot be empty".to_string(),
                ));
            }
        }
        if let Some(ref name) = self.last_name {
            if name.trim().is_empty() {
                return Err(PartyError::Validation(
                    "last_name cannot be empty".to_string(),
                ));
            }
        }
        if let Some(ref email) = self.email {
            if !email.contains('@') || email.trim().is_empty() {
                return Err(PartyError::Validation(
                    "email must be a valid email address".to_string(),
                ));
            }
        }
        Ok(())
    }
}
