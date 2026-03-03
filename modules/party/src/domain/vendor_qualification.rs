//! Vendor qualification domain types — compliance gate for aerospace procurement.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::party::PartyError;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct VendorQualification {
    pub id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub qualification_status: String,
    pub certification_ref: Option<String>,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
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
pub struct CreateVendorQualificationRequest {
    pub qualification_status: String,
    pub certification_ref: Option<String>,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateVendorQualificationRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.qualification_status.trim().is_empty() {
            return Err(PartyError::Validation(
                "qualification_status cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Update Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateVendorQualificationRequest {
    pub qualification_status: Option<String>,
    pub certification_ref: Option<String>,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl UpdateVendorQualificationRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref status) = self.qualification_status {
            if status.trim().is_empty() {
                return Err(PartyError::Validation(
                    "qualification_status cannot be empty".to_string(),
                ));
            }
        }
        Ok(())
    }
}
