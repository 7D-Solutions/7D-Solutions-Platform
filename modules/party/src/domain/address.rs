//! Address domain types — typed addresses linked to a party.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::party::PartyError;

// ============================================================================
// Address Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Address {
    pub id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    #[sqlx(rename = "address_type")]
    pub address_type: String,
    pub label: Option<String>,
    pub line1: String,
    pub line2: Option<String>,
    pub city: String,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: String,
    pub is_primary: bool,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Address type validation
// ============================================================================

const VALID_ADDRESS_TYPES: &[&str] = &["billing", "shipping", "registered", "mailing", "other"];

fn validate_address_type(t: &str) -> Result<(), PartyError> {
    if VALID_ADDRESS_TYPES.contains(&t) {
        Ok(())
    } else {
        Err(PartyError::Validation(format!(
            "address_type must be one of: {}",
            VALID_ADDRESS_TYPES.join(", ")
        )))
    }
}

// ============================================================================
// Create Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAddressRequest {
    pub address_type: Option<String>,
    pub label: Option<String>,
    pub line1: String,
    pub line2: Option<String>,
    pub city: String,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub is_primary: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateAddressRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.line1.trim().is_empty() {
            return Err(PartyError::Validation(
                "line1 cannot be empty".to_string(),
            ));
        }
        if self.city.trim().is_empty() {
            return Err(PartyError::Validation(
                "city cannot be empty".to_string(),
            ));
        }
        if let Some(ref t) = self.address_type {
            validate_address_type(t)?;
        }
        Ok(())
    }
}

// ============================================================================
// Update Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAddressRequest {
    pub address_type: Option<String>,
    pub label: Option<String>,
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub is_primary: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

impl UpdateAddressRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref line) = self.line1 {
            if line.trim().is_empty() {
                return Err(PartyError::Validation(
                    "line1 cannot be empty".to_string(),
                ));
            }
        }
        if let Some(ref city) = self.city {
            if city.trim().is_empty() {
                return Err(PartyError::Validation(
                    "city cannot be empty".to_string(),
                ));
            }
        }
        if let Some(ref t) = self.address_type {
            validate_address_type(t)?;
        }
        Ok(())
    }
}
