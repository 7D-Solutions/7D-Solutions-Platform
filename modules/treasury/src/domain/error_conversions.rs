use platform_http_contracts::ApiError;

use super::accounts::AccountError;
use super::import::ImportError;
use super::recon::ReconError;

impl From<AccountError> for ApiError {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::NotFound(id) => {
                ApiError::not_found(format!("Treasury account {} not found", id))
            }
            AccountError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            AccountError::IdempotentReplay { .. } => {
                ApiError::new(200, "idempotent_replay", "Request already processed")
            }
            AccountError::Database(e) => {
                tracing::error!(error = %e, "treasury accounts database error");
                ApiError::internal("Database error")
            }
        }
    }
}

impl From<ReconError> for ApiError {
    fn from(err: ReconError) -> Self {
        match err {
            ReconError::StatementLineNotFound(id) => {
                ApiError::not_found(format!("Statement line {} not found", id))
            }
            ReconError::TransactionNotFound(id) => {
                ApiError::not_found(format!("Bank transaction {} not found", id))
            }
            ReconError::MatchNotFound(id) => {
                ApiError::not_found(format!("Recon match {} not found", id))
            }
            ReconError::AmountMismatch {
                stmt_amount,
                txn_amount,
            } => ApiError::new(
                422,
                "amount_mismatch",
                format!(
                    "Statement amount {} does not match transaction amount {}",
                    stmt_amount, txn_amount
                ),
            ),
            ReconError::CurrencyMismatch {
                stmt_currency,
                txn_currency,
            } => ApiError::new(
                422,
                "currency_mismatch",
                format!("Currency mismatch: {} vs {}", stmt_currency, txn_currency),
            ),
            ReconError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ReconError::Database(e) => {
                tracing::error!(error = %e, "treasury recon database error");
                ApiError::internal("Database error")
            }
        }
    }
}

impl From<ImportError> for ApiError {
    fn from(err: ImportError) -> Self {
        match err {
            ImportError::AccountNotFound(id) => {
                ApiError::not_found(format!("Bank account {} not found", id))
            }
            ImportError::AccountNotActive => {
                ApiError::new(422, "account_not_active", "Bank account is not active")
            }
            ImportError::DuplicateImport { statement_id } => ApiError::new(
                200,
                "duplicate_import",
                format!(
                    "Statement already imported with id {}. No duplicates created.",
                    statement_id
                ),
            ),
            ImportError::EmptyImport => {
                ApiError::new(422, "empty_import", "CSV contains no transaction lines")
            }
            ImportError::AllLinesFailed(_) => ApiError::new(
                422,
                "all_lines_failed",
                "Every CSV line failed validation",
            ),
            ImportError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ImportError::Database(e) => {
                tracing::error!(error = %e, "treasury import database error");
                ApiError::internal("Database error")
            }
        }
    }
}
