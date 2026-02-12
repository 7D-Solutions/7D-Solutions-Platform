//! Journal entry business logic service
//!
//! This service handles the creation of journal entries from GL posting requests,
//! ensuring transactional consistency and validation.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::contracts::gl_posting_request_v1::GlPostingRequestV1;
use crate::repos::{journal_repo, processed_repo};
use crate::validation::{validate_gl_posting_request, ValidationError};

/// Errors that can occur during journal entry processing
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("Validation failed: {0}")]
    Validation(#[from] ValidationError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Invalid posting date: {0}")]
    InvalidDate(String),

    #[error("Event already processed (duplicate): {0}")]
    DuplicateEvent(Uuid),
}

/// Result type for journal operations
pub type JournalResult<T> = Result<T, JournalError>;

/// Process a GL posting request and create a journal entry
///
/// This function:
/// 1. Checks for duplicate events (idempotency)
/// 2. Validates the payload
/// 3. Creates journal entry and lines in a transaction
/// 4. Marks the event as processed
///
/// All operations are wrapped in a single transaction for atomicity.
pub async fn process_gl_posting_request(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    source_subject: &str,
    payload: &GlPostingRequestV1,
) -> JournalResult<Uuid> {
    // Check if event already processed (idempotency)
    if processed_repo::exists(pool, event_id).await? {
        tracing::info!(
            event_id = %event_id,
            "Event already processed, skipping (idempotency)"
        );
        return Err(JournalError::DuplicateEvent(event_id));
    }

    // Validate the payload
    validate_gl_posting_request(payload)?;

    // Parse posting date
    let posting_date = NaiveDate::parse_from_str(&payload.posting_date, "%Y-%m-%d")
        .map_err(|e| JournalError::InvalidDate(format!("{}: {}", payload.posting_date, e)))?;
    let posted_at: DateTime<Utc> = posting_date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| JournalError::InvalidDate("Invalid time".to_string()))?
        .and_utc();

    // Start transaction
    let mut tx = pool.begin().await?;

    // Generate journal entry ID
    let entry_id = Uuid::new_v4();

    // Insert journal entry header
    journal_repo::insert_entry(
        &mut tx,
        entry_id,
        tenant_id,
        source_module,
        event_id,
        source_subject,
        posted_at,
        &payload.currency,
        Some(&payload.description),
        Some(&payload.source_doc_type.to_string()),
        Some(&payload.source_doc_id),
    )
    .await?;

    // Convert payload lines to repo insert format
    let lines: Vec<journal_repo::JournalLineInsert> = payload
        .lines
        .iter()
        .enumerate()
        .map(|(idx, line)| journal_repo::JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: (idx + 1) as i32,
            account_ref: line.account_ref.clone(),
            debit_minor: (line.debit * 100.0).round() as i64, // Convert to minor units
            credit_minor: (line.credit * 100.0).round() as i64,
            memo: line.memo.clone(),
        })
        .collect();

    // Insert journal lines
    journal_repo::bulk_insert_lines(&mut tx, entry_id, lines).await?;

    // Mark event as processed
    processed_repo::insert(&mut tx, event_id, "gl.events.posting.requested", "gl-consumer")
        .await?;

    // Commit transaction
    tx.commit().await?;

    tracing::info!(
        event_id = %event_id,
        entry_id = %entry_id,
        tenant_id = %tenant_id,
        "Journal entry created successfully"
    );

    Ok(entry_id)
}

/// Helper to convert SourceDocType to string
impl std::fmt::Display for crate::contracts::gl_posting_request_v1::SourceDocType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArInvoice => write!(f, "AR_INVOICE"),
            Self::ArPayment => write!(f, "AR_PAYMENT"),
            Self::ArCreditMemo => write!(f, "AR_CREDIT_MEMO"),
            Self::ArAdjustment => write!(f, "AR_ADJUSTMENT"),
            Self::ApBill => write!(f, "AP_BILL"),
            Self::ApPayment => write!(f, "AP_PAYMENT"),
            Self::InventoryReceipt => write!(f, "INVENTORY_RECEIPT"),
            Self::InventoryIssue => write!(f, "INVENTORY_ISSUE"),
            Self::PayrollRun => write!(f, "PAYROLL_RUN"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::gl_posting_request_v1::{JournalLine, SourceDocType};

    fn create_test_payload() -> GlPostingRequestV1 {
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
                    memo: Some("Accounts Receivable".to_string()),
                    dimensions: None,
                },
                JournalLine {
                    account_ref: "4000".to_string(),
                    debit: 0.0,
                    credit: 100.0,
                    memo: Some("Revenue".to_string()),
                    dimensions: None,
                },
            ],
        }
    }

    #[test]
    fn test_validation_error_handling() {
        let mut payload = create_test_payload();
        payload.lines[0].debit = -50.0; // Invalid negative debit

        let result = validate_gl_posting_request(&payload);
        assert!(result.is_err());
    }
}
