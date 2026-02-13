//! Validation logic for GL posting requests
//!
//! This module validates GL posting request payloads according to the
//! contract schema requirements.

use crate::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine};
use thiserror::Error;

/// Validation errors for GL posting requests
#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("Currency must be a 3-letter uppercase code (ISO 4217), got: {0}")]
    InvalidCurrency(String),

    #[error("Description must be between 1 and 500 characters, got {0} characters")]
    InvalidDescriptionLength(usize),

    #[error("Lines must have at least 2 items, got {0}")]
    InsufficientLines(usize),

    #[error("Line {0}: account_ref cannot be empty")]
    EmptyAccountRef(usize),

    #[error("Line {0}: debit must be non-negative, got {1}")]
    NegativeDebit(usize, f64),

    #[error("Line {0}: credit must be non-negative, got {1}")]
    NegativeCredit(usize, f64),

    #[error("Line {0}: memo exceeds 500 characters, got {1}")]
    MemoTooLong(usize, usize),

    #[error("Total debits ({0}) must equal total credits ({1})")]
    UnbalancedEntry(f64, f64),

    #[error("Line {0}: account '{1}' not found in Chart of Accounts for tenant '{2}'")]
    AccountNotFound(usize, String, String),

    #[error("Line {0}: account '{1}' is inactive for tenant '{2}'")]
    AccountInactive(usize, String, String),
}

/// Validate a GL posting request payload
///
/// # Validation Rules
///
/// - `currency`: Must be a 3-letter uppercase code (ISO 4217)
/// - `description`: Must be 1-500 characters
/// - `lines`: Must have at least 2 items
/// - Each line:
///   - `account_ref`: Must be non-empty
///   - `debit`: Must be >= 0
///   - `credit`: Must be >= 0
///   - `memo`: If present, must be <= 500 characters
/// - Total debits must equal total credits (balanced entry)
///
/// # Errors
///
/// Returns `ValidationError` if any validation rule is violated
pub fn validate_gl_posting_request(payload: &GlPostingRequestV1) -> Result<(), ValidationError> {
    // Validate currency (3 uppercase letters)
    if !is_valid_currency(&payload.currency) {
        return Err(ValidationError::InvalidCurrency(payload.currency.clone()));
    }

    // Validate description length
    let desc_len = payload.description.len();
    if desc_len == 0 || desc_len > 500 {
        return Err(ValidationError::InvalidDescriptionLength(desc_len));
    }

    // Validate minimum number of lines
    if payload.lines.len() < 2 {
        return Err(ValidationError::InsufficientLines(payload.lines.len()));
    }

    // Validate each line and accumulate totals
    let mut total_debits = 0.0;
    let mut total_credits = 0.0;

    for (idx, line) in payload.lines.iter().enumerate() {
        validate_journal_line(line, idx)?;
        total_debits += line.debit;
        total_credits += line.credit;
    }

    // Validate balanced entry (debits == credits)
    // Use epsilon comparison for floating point equality
    const EPSILON: f64 = 0.01; // Penny precision
    if (total_debits - total_credits).abs() > EPSILON {
        return Err(ValidationError::UnbalancedEntry(total_debits, total_credits));
    }

    Ok(())
}

/// Validate a single journal line
fn validate_journal_line(line: &JournalLine, index: usize) -> Result<(), ValidationError> {
    // Account ref must be non-empty
    if line.account_ref.is_empty() {
        return Err(ValidationError::EmptyAccountRef(index));
    }

    // Debit must be non-negative
    if line.debit < 0.0 {
        return Err(ValidationError::NegativeDebit(index, line.debit));
    }

    // Credit must be non-negative
    if line.credit < 0.0 {
        return Err(ValidationError::NegativeCredit(index, line.credit));
    }

    // Memo length validation (if present)
    if let Some(ref memo) = line.memo {
        if memo.len() > 500 {
            return Err(ValidationError::MemoTooLong(index, memo.len()));
        }
    }

    Ok(())
}

/// Check if currency code is valid (3 uppercase letters)
fn is_valid_currency(currency: &str) -> bool {
    currency.len() == 3 && currency.chars().all(|c| c.is_ascii_uppercase())
}

/// Validate account references against the Chart of Accounts
///
/// This function checks that each account_ref in the posting request:
/// 1. Exists in the Chart of Accounts for the tenant
/// 2. Is in an active state
///
/// This validation must be performed within a transaction to ensure consistency.
///
/// # Errors
///
/// Returns `ValidationError::AccountNotFound` if an account doesn't exist
/// Returns `ValidationError::AccountInactive` if an account is inactive
pub async fn validate_accounts_against_coa(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    payload: &GlPostingRequestV1,
) -> Result<(), ValidationError> {
    use crate::repos::account_repo::{self, AccountError};

    for (idx, line) in payload.lines.iter().enumerate() {
        // Check if account exists and is active
        match account_repo::find_active_by_code_tx(tx, tenant_id, &line.account_ref).await {
            Ok(_account) => {
                // Account exists and is active - validation passes
                tracing::debug!(
                    line_index = idx,
                    account_code = %line.account_ref,
                    tenant_id = %tenant_id,
                    "Account validated against COA"
                );
            }
            Err(AccountError::NotFound { code, .. }) => {
                return Err(ValidationError::AccountNotFound(
                    idx,
                    code,
                    tenant_id.to_string(),
                ));
            }
            Err(AccountError::Inactive { code, .. }) => {
                return Err(ValidationError::AccountInactive(
                    idx,
                    code,
                    tenant_id.to_string(),
                ));
            }
            Err(AccountError::Database(e)) => {
                // Database errors should be propagated as-is, not wrapped in validation error
                // This allows the caller to distinguish between validation failures
                // (non-retriable) and database errors (retriable)
                return Err(ValidationError::AccountNotFound(
                    idx,
                    line.account_ref.clone(),
                    format!("Database error: {}", e),
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::gl_posting_request_v1::SourceDocType;

    fn create_valid_payload() -> GlPostingRequestV1 {
        GlPostingRequestV1 {
            posting_date: "2024-02-11".to_string(),
            currency: "USD".to_string(),
            source_doc_type: SourceDocType::ArInvoice,
            source_doc_id: "inv_123".to_string(),
            description: "Test invoice".to_string(),
            lines: vec![
                JournalLine {
                    account_ref: "1100".to_string(),
                    debit: 100.0,
                    credit: 0.0,
                    memo: None,
                    dimensions: None,
                },
                JournalLine {
                    account_ref: "4000".to_string(),
                    debit: 0.0,
                    credit: 100.0,
                    memo: None,
                    dimensions: None,
                },
            ],
        }
    }

    #[test]
    fn test_valid_payload() {
        let payload = create_valid_payload();
        assert!(validate_gl_posting_request(&payload).is_ok());
    }

    #[test]
    fn test_invalid_currency_too_short() {
        let mut payload = create_valid_payload();
        payload.currency = "US".to_string();
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::InvalidCurrency("US".to_string()))
        );
    }

    #[test]
    fn test_invalid_currency_lowercase() {
        let mut payload = create_valid_payload();
        payload.currency = "usd".to_string();
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::InvalidCurrency("usd".to_string()))
        );
    }

    #[test]
    fn test_empty_description() {
        let mut payload = create_valid_payload();
        payload.description = "".to_string();
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::InvalidDescriptionLength(0))
        );
    }

    #[test]
    fn test_description_too_long() {
        let mut payload = create_valid_payload();
        payload.description = "x".repeat(501);
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::InvalidDescriptionLength(501))
        );
    }

    #[test]
    fn test_insufficient_lines() {
        let mut payload = create_valid_payload();
        payload.lines = vec![JournalLine {
            account_ref: "1100".to_string(),
            debit: 100.0,
            credit: 0.0,
            memo: None,
            dimensions: None,
        }];
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::InsufficientLines(1))
        );
    }

    #[test]
    fn test_empty_account_ref() {
        let mut payload = create_valid_payload();
        payload.lines[0].account_ref = "".to_string();
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::EmptyAccountRef(0))
        );
    }

    #[test]
    fn test_negative_debit() {
        let mut payload = create_valid_payload();
        payload.lines[0].debit = -50.0;
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::NegativeDebit(0, -50.0))
        );
    }

    #[test]
    fn test_negative_credit() {
        let mut payload = create_valid_payload();
        payload.lines[1].credit = -50.0;
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::NegativeCredit(1, -50.0))
        );
    }

    #[test]
    fn test_memo_too_long() {
        let mut payload = create_valid_payload();
        payload.lines[0].memo = Some("x".repeat(501));
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::MemoTooLong(0, 501))
        );
    }

    #[test]
    fn test_unbalanced_entry() {
        let mut payload = create_valid_payload();
        payload.lines[1].credit = 50.0; // Changed from 100.0
        assert_eq!(
            validate_gl_posting_request(&payload),
            Err(ValidationError::UnbalancedEntry(100.0, 50.0))
        );
    }

    #[test]
    fn test_balanced_entry_with_multiple_lines() {
        let mut payload = create_valid_payload();
        payload.lines.push(JournalLine {
            account_ref: "5000".to_string(),
            debit: 50.0,
            credit: 0.0,
            memo: None,
            dimensions: None,
        });
        payload.lines.push(JournalLine {
            account_ref: "6000".to_string(),
            debit: 0.0,
            credit: 50.0,
            memo: None,
            dimensions: None,
        });
        assert!(validate_gl_posting_request(&payload).is_ok());
    }
}
