//! Reversal service for creating inverse journal entries
//!
//! This service handles the creation of reversal entries that undo
//! the effect of original journal entries by creating inverse entries.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::contracts::gl_entry_reverse_request_v1::GlEntryReversedV1;
use crate::repos::{balance_repo, journal_repo, outbox_repo, period_repo, processed_repo};
use crate::services::{balance_deltas::JournalLineInput, balance_updater};

/// Errors that can occur during reversal operations
#[derive(Debug, thiserror::Error)]
pub enum ReversalError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Original entry not found: {0}")]
    EntryNotFound(Uuid),

    #[error("Entry already reversed: {0}")]
    AlreadyReversed(Uuid),

    #[error("Cannot reverse entry {original_entry_id} - original period {period_id} was closed")]
    OriginalPeriodClosed {
        original_entry_id: Uuid,
        period_id: Uuid,
    },

    #[error("Event already processed (duplicate): {0}")]
    DuplicateEvent(Uuid),

    #[error("Period validation failed: {0}")]
    Period(#[from] period_repo::PeriodError),

    #[error("Balance update failed: {0}")]
    Balance(#[from] balance_repo::BalanceError),
}

/// Result type for reversal operations
pub type ReversalResult<T> = Result<T, ReversalError>;

/// Create a reversal entry for an existing journal entry
///
/// This function:
/// 1. Checks for duplicate reversal events (idempotency)
/// 2. Loads the original journal entry with its lines
/// 3. Creates an inverse entry with swapped debit/credit
/// 4. Sets reverses_entry_id to link back to original
/// 5. Marks the reversal event as processed
///
/// All operations are wrapped in a single transaction for atomicity.
pub async fn create_reversal_entry(
    pool: &PgPool,
    reversal_event_id: Uuid,
    original_entry_id: Uuid,
) -> ReversalResult<Uuid> {
    // Check if reversal event already processed (idempotency)
    if processed_repo::exists(pool, reversal_event_id).await? {
        tracing::info!(
            reversal_event_id = %reversal_event_id,
            "Reversal event already processed, skipping (idempotency)"
        );
        return Err(ReversalError::DuplicateEvent(reversal_event_id));
    }

    // Load original entry with lines
    let (original_entry, original_lines) = journal_repo::fetch_entry_with_lines(pool, original_entry_id)
        .await?
        .ok_or(ReversalError::EntryNotFound(original_entry_id))?;

    // Check if entry is already a reversal
    if original_entry.reverses_entry_id.is_some() {
        tracing::warn!(
            original_entry_id = %original_entry_id,
            "Attempting to reverse an entry that is itself a reversal"
        );
        return Err(ReversalError::AlreadyReversed(original_entry_id));
    }

    // Phase 13: Check if original entry's period is closed
    // If the original transaction's period is closed, we cannot reverse it
    let original_entry_date = original_entry.posted_at.date_naive();
    let original_period = period_repo::find_by_date(pool, &original_entry.tenant_id, original_entry_date)
        .await?
        .ok_or_else(|| {
            period_repo::PeriodError::NoPeriodForDate {
                tenant_id: original_entry.tenant_id.clone(),
                date: original_entry_date,
            }
        })?;

    if original_period.closed_at.is_some() {
        tracing::warn!(
            original_entry_id = %original_entry_id,
            original_period_id = %original_period.id,
            "Cannot reverse entry - original period is closed"
        );
        return Err(ReversalError::OriginalPeriodClosed {
            original_entry_id,
            period_id: original_period.id,
        });
    }

    // Start transaction
    let mut tx = pool.begin().await?;

    // Get the accounting period for the reversal date
    let reversal_date = Utc::now().date_naive();
    let period = period_repo::find_by_date_tx(&mut tx, &original_entry.tenant_id, reversal_date)
        .await?
        .ok_or_else(|| {
            period_repo::PeriodError::NoPeriodForDate {
                tenant_id: original_entry.tenant_id.clone(),
                date: reversal_date,
            }
        })?;

    // Verify reversal period is not closed (Phase 13: use closed_at semantics)
    if period.closed_at.is_some() {
        return Err(ReversalError::Period(period_repo::PeriodError::PeriodClosed {
            tenant_id: original_entry.tenant_id.clone(),
            date: reversal_date,
            period_id: period.id,
        }));
    }

    let period_id = period.id;

    // Generate new entry ID for reversal
    let reversal_entry_id = Uuid::new_v4();

    // Create reversal entry header
    journal_repo::insert_entry_with_reversal(
        &mut tx,
        reversal_entry_id,
        &original_entry.tenant_id,
        &original_entry.source_module,
        reversal_event_id,
        &format!("REVERSAL: {}", original_entry.source_subject),
        Utc::now(), // Use current timestamp for reversal
        &original_entry.currency,
        Some(&format!(
            "Reversal of journal entry {}",
            original_entry_id
        )),
        original_entry.reference_type.as_deref(),
        original_entry.reference_id.as_deref(),
        Some(original_entry_id), // Link back to original
    )
    .await?;

    // Create inverse journal lines (swap debit/credit)
    let reversal_lines: Vec<journal_repo::JournalLineInsert> = original_lines
        .iter()
        .map(|line| journal_repo::JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: line.line_no,
            account_ref: line.account_ref.clone(),
            debit_minor: line.credit_minor, // Swap: credit becomes debit
            credit_minor: line.debit_minor,  // Swap: debit becomes credit
            memo: line.memo.as_ref().map(|m| format!("REVERSAL: {}", m)),
        })
        .collect();

    // Insert reversal lines
    journal_repo::bulk_insert_lines(&mut tx, reversal_entry_id, reversal_lines.clone()).await?;

    // Update account balances from reversal lines (exactly-once, within same transaction)
    // Convert reversal lines to balance delta input format
    let balance_input: Vec<JournalLineInput> = reversal_lines
        .iter()
        .map(|line| JournalLineInput {
            account_ref: line.account_ref.clone(),
            debit_minor: line.debit_minor,
            credit_minor: line.credit_minor,
        })
        .collect();

    // Update balances atomically within the reversal transaction
    balance_updater::update_balances_from_journal(
        &mut tx,
        &original_entry.tenant_id,
        period_id,
        &original_entry.currency,
        reversal_entry_id,
        &balance_input,
    )
    .await?;

    // Mark reversal event as processed
    processed_repo::insert(
        &mut tx,
        reversal_event_id,
        "gl.events.entry.reverse.requested",
        "gl-reversal-service",
    )
    .await?;

    // Emit gl.events.entry.reversed event
    let reversed_event = GlEntryReversedV1 {
        original_entry_id,
        reversal_entry_id,
        currency: original_entry.currency.clone(),
        posted_at: Some(Utc::now().to_rfc3339()),
    };

    let reversed_event_id = Uuid::new_v4();
    outbox_repo::insert_outbox_event(
        &mut tx,
        reversed_event_id,
        "gl.events.entry.reversed",
        "journal_entry",
        &reversal_entry_id.to_string(),
        serde_json::to_value(&reversed_event)
            .map_err(|e| sqlx::Error::Protocol(format!("JSON serialization failed: {}", e)))?,
    )
    .await?;

    // Commit transaction
    tx.commit().await?;

    tracing::info!(
        reversal_event_id = %reversal_event_id,
        reversal_entry_id = %reversal_entry_id,
        original_entry_id = %original_entry_id,
        tenant_id = %original_entry.tenant_id,
        "Reversal entry created successfully"
    );

    Ok(reversal_entry_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reversal_error_display() {
        let err = ReversalError::EntryNotFound(Uuid::new_v4());
        assert!(err.to_string().contains("not found"));
    }
}
