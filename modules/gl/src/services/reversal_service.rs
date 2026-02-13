//! Reversal service for creating inverse journal entries
//!
//! This service handles the creation of reversal entries that undo
//! the effect of original journal entries by creating inverse entries.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::contracts::gl_entry_reverse_request_v1::GlEntryReversedV1;
use crate::repos::{journal_repo, outbox_repo, processed_repo};

/// Errors that can occur during reversal operations
#[derive(Debug, thiserror::Error)]
pub enum ReversalError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Original entry not found: {0}")]
    EntryNotFound(Uuid),

    #[error("Entry already reversed: {0}")]
    AlreadyReversed(Uuid),

    #[error("Event already processed (duplicate): {0}")]
    DuplicateEvent(Uuid),
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

    // Start transaction
    let mut tx = pool.begin().await?;

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
    journal_repo::bulk_insert_lines(&mut tx, reversal_entry_id, reversal_lines).await?;

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
