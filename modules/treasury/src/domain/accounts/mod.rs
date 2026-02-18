//! Bank accounts bounded context — types, validation, and error handling.
//!
//! Bank accounts are app_id-scoped. No secrets stored. Balances are
//! informational (updated externally via statement import or ingest consumers).

pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("Bank account not found: {0}")]
    NotFound(Uuid),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotent replay: request already processed")]
    IdempotentReplay { status_code: u16, body: serde_json::Value },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Status Enum
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "treasury_account_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    Inactive,
    Closed,
}

// ============================================================================
// Domain Structs
// ============================================================================

/// Full bank account record as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BankAccount {
    pub id: Uuid,
    pub app_id: String,
    pub account_name: String,
    pub institution: Option<String>,
    /// Last 4 digits only — no full account numbers stored.
    pub account_number_last4: Option<String>,
    pub routing_number: Option<String>,
    /// ISO 4217 currency code (e.g. "USD")
    pub currency: String,
    /// Informational balance in minor units; updated by statement import.
    pub current_balance_minor: i64,
    pub status: AccountStatus,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body to create a bank account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAccountRequest {
    pub account_name: String,
    pub institution: Option<String>,
    /// Last 4 digits only. Reject anything longer.
    pub account_number_last4: Option<String>,
    pub routing_number: Option<String>,
    /// ISO 4217 currency code — must be exactly 3 uppercase letters.
    pub currency: String,
    pub metadata: Option<serde_json::Value>,
}

/// Request body for partial updates to a bank account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAccountRequest {
    pub account_name: Option<String>,
    pub institution: Option<String>,
    pub account_number_last4: Option<String>,
    pub routing_number: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

// ============================================================================
// Validation
// ============================================================================

impl CreateAccountRequest {
    pub fn validate(&self) -> Result<(), AccountError> {
        if self.account_name.trim().is_empty() {
            return Err(AccountError::Validation(
                "account_name cannot be empty".to_string(),
            ));
        }
        if self.currency.trim().len() != 3
            || !self.currency.chars().all(|c| c.is_ascii_alphabetic())
        {
            return Err(AccountError::Validation(
                "currency must be a 3-character ISO 4217 code (e.g. USD)".to_string(),
            ));
        }
        if let Some(ref last4) = self.account_number_last4 {
            if last4.len() > 4 || !last4.chars().all(|c| c.is_ascii_digit()) {
                return Err(AccountError::Validation(
                    "account_number_last4 must be up to 4 digits".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl UpdateAccountRequest {
    pub fn validate(&self) -> Result<(), AccountError> {
        if let Some(ref name) = self.account_name {
            if name.trim().is_empty() {
                return Err(AccountError::Validation(
                    "account_name cannot be empty".to_string(),
                ));
            }
        }
        if let Some(ref last4) = self.account_number_last4 {
            if last4.len() > 4 || !last4.chars().all(|c| c.is_ascii_digit()) {
                return Err(AccountError::Validation(
                    "account_number_last4 must be up to 4 digits".to_string(),
                ));
            }
        }
        Ok(())
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_create() -> CreateAccountRequest {
        CreateAccountRequest {
            account_name: "Checking".to_string(),
            institution: Some("First Bank".to_string()),
            account_number_last4: Some("1234".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        }
    }

    #[test]
    fn create_valid() {
        assert!(base_create().validate().is_ok());
    }

    #[test]
    fn create_empty_name_rejected() {
        let mut req = base_create();
        req.account_name = "  ".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn create_bad_currency_rejected() {
        let mut req = base_create();
        req.currency = "US".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn create_numeric_currency_rejected() {
        let mut req = base_create();
        req.currency = "123".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn create_last4_too_long_rejected() {
        let mut req = base_create();
        req.account_number_last4 = Some("12345".to_string());
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn create_last4_non_digit_rejected() {
        let mut req = base_create();
        req.account_number_last4 = Some("123X".to_string());
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn update_empty_name_rejected() {
        let req = UpdateAccountRequest {
            account_name: Some("  ".to_string()),
            institution: None,
            account_number_last4: None,
            routing_number: None,
            metadata: None,
        };
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn update_all_none_is_valid() {
        let req = UpdateAccountRequest {
            account_name: None,
            institution: None,
            account_number_last4: None,
            routing_number: None,
            metadata: None,
        };
        assert!(req.validate().is_ok());
    }
}
