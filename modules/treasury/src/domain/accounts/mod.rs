//! Treasury accounts bounded context — types, validation, and error handling.
//!
//! Supports both bank accounts (routing/ACH) and credit card accounts
//! (credit limit, statement closing day, network). All accounts are
//! app_id-scoped. No secrets stored. Balances are informational.

pub mod repo;
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
    #[error("Treasury account not found: {0}")]
    NotFound(Uuid),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotent replay: request already processed")]
    IdempotentReplay {
        status_code: u16,
        body: serde_json::Value,
    },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Enums
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema)]
#[sqlx(type_name = "treasury_account_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    Inactive,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema)]
#[sqlx(type_name = "treasury_account_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    Bank,
    CreditCard,
}

// ============================================================================
// Domain Structs
// ============================================================================

/// Full treasury account record as stored and returned.
/// Covers both bank and credit card accounts; CC-specific fields are None
/// for bank accounts and vice-versa.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TreasuryAccount {
    pub id: Uuid,
    pub app_id: String,
    pub account_name: String,
    pub account_type: AccountType,
    pub institution: Option<String>,
    /// Last 4 digits only — no full account numbers stored.
    pub account_number_last4: Option<String>,
    // Bank-specific
    pub routing_number: Option<String>,
    /// ISO 4217 currency code (e.g. "USD")
    pub currency: String,
    /// Informational balance in minor units; updated by statement import.
    pub current_balance_minor: i64,
    pub status: AccountStatus,
    // Credit card specific (None for bank accounts)
    pub credit_limit_minor: Option<i64>,
    /// Day of month (1–31) on which CC statement closes.
    pub statement_closing_day: Option<i32>,
    /// Card network: Visa, Mastercard, Amex, Discover, etc.
    pub cc_network: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body to create a bank account.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateBankAccountRequest {
    pub account_name: String,
    pub institution: Option<String>,
    /// Last 4 digits only. Reject anything longer.
    pub account_number_last4: Option<String>,
    pub routing_number: Option<String>,
    /// ISO 4217 currency code — must be exactly 3 uppercase letters.
    pub currency: String,
    pub metadata: Option<serde_json::Value>,
}

/// Request body to create a credit card account.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateCreditCardAccountRequest {
    pub account_name: String,
    pub institution: Option<String>,
    /// Last 4 digits of card number.
    pub account_number_last4: Option<String>,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Credit limit in minor units (e.g. 500_000 = $5,000.00 USD).
    pub credit_limit_minor: Option<i64>,
    /// Day of month (1–31) the billing cycle closes.
    pub statement_closing_day: Option<i32>,
    /// Card network identifier (e.g. "Visa", "Mastercard", "Amex").
    pub cc_network: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Request body for partial updates to a treasury account.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateAccountRequest {
    pub account_name: Option<String>,
    pub institution: Option<String>,
    pub account_number_last4: Option<String>,
    pub routing_number: Option<String>,
    pub credit_limit_minor: Option<i64>,
    pub statement_closing_day: Option<i32>,
    pub cc_network: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

// ============================================================================
// Validation
// ============================================================================

impl CreateBankAccountRequest {
    pub fn validate(&self) -> Result<(), AccountError> {
        validate_account_name(&self.account_name)?;
        validate_currency(&self.currency)?;
        validate_last4(self.account_number_last4.as_deref())?;
        Ok(())
    }
}

impl CreateCreditCardAccountRequest {
    pub fn validate(&self) -> Result<(), AccountError> {
        validate_account_name(&self.account_name)?;
        validate_currency(&self.currency)?;
        validate_last4(self.account_number_last4.as_deref())?;
        if let Some(day) = self.statement_closing_day {
            if !(1..=31).contains(&day) {
                return Err(AccountError::Validation(
                    "statement_closing_day must be between 1 and 31".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl UpdateAccountRequest {
    pub fn validate(&self) -> Result<(), AccountError> {
        if let Some(ref name) = self.account_name {
            validate_account_name(name)?;
        }
        validate_last4(self.account_number_last4.as_deref())?;
        if let Some(day) = self.statement_closing_day {
            if !(1..=31).contains(&day) {
                return Err(AccountError::Validation(
                    "statement_closing_day must be between 1 and 31".to_string(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_account_name(name: &str) -> Result<(), AccountError> {
    if name.trim().is_empty() {
        return Err(AccountError::Validation(
            "account_name cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_currency(currency: &str) -> Result<(), AccountError> {
    if currency.trim().len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(AccountError::Validation(
            "currency must be a 3-character ISO 4217 code (e.g. USD)".to_string(),
        ));
    }
    Ok(())
}

fn validate_last4(last4: Option<&str>) -> Result<(), AccountError> {
    if let Some(v) = last4 {
        if v.len() > 4 || !v.chars().all(|c| c.is_ascii_digit()) {
            return Err(AccountError::Validation(
                "account_number_last4 must be up to 4 digits".to_string(),
            ));
        }
    }
    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_bank() -> CreateBankAccountRequest {
        CreateBankAccountRequest {
            account_name: "Checking".to_string(),
            institution: Some("First Bank".to_string()),
            account_number_last4: Some("1234".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        }
    }

    fn base_cc() -> CreateCreditCardAccountRequest {
        CreateCreditCardAccountRequest {
            account_name: "Corp Visa".to_string(),
            institution: Some("Chase".to_string()),
            account_number_last4: Some("9876".to_string()),
            currency: "USD".to_string(),
            credit_limit_minor: Some(500_000),
            statement_closing_day: Some(15),
            cc_network: Some("Visa".to_string()),
            metadata: None,
        }
    }

    #[test]
    fn bank_create_valid() {
        assert!(base_bank().validate().is_ok());
    }

    #[test]
    fn bank_empty_name_rejected() {
        let mut req = base_bank();
        req.account_name = "  ".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn bank_bad_currency_rejected() {
        let mut req = base_bank();
        req.currency = "US".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn bank_numeric_currency_rejected() {
        let mut req = base_bank();
        req.currency = "123".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn bank_last4_too_long_rejected() {
        let mut req = base_bank();
        req.account_number_last4 = Some("12345".to_string());
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn cc_create_valid() {
        assert!(base_cc().validate().is_ok());
    }

    #[test]
    fn cc_empty_name_rejected() {
        let mut req = base_cc();
        req.account_name = "".to_string();
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn cc_closing_day_zero_rejected() {
        let mut req = base_cc();
        req.statement_closing_day = Some(0);
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn cc_closing_day_32_rejected() {
        let mut req = base_cc();
        req.statement_closing_day = Some(32);
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn cc_no_closing_day_is_valid() {
        let mut req = base_cc();
        req.statement_closing_day = None;
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_empty_name_rejected() {
        let req = UpdateAccountRequest {
            account_name: Some("  ".to_string()),
            institution: None,
            account_number_last4: None,
            routing_number: None,
            credit_limit_minor: None,
            statement_closing_day: None,
            cc_network: None,
            metadata: None,
        };
        assert!(matches!(req.validate(), Err(AccountError::Validation(_))));
    }

    #[test]
    fn update_invalid_closing_day_rejected() {
        let req = UpdateAccountRequest {
            account_name: None,
            institution: None,
            account_number_last4: None,
            routing_number: None,
            credit_limit_minor: None,
            statement_closing_day: Some(0),
            cc_network: None,
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
            credit_limit_minor: None,
            statement_closing_day: None,
            cc_network: None,
            metadata: None,
        };
        assert!(req.validate().is_ok());
    }
}
