//! Balance updater service
//!
//! This service orchestrates balance updates within posting transactions,
//! ensuring exactly-once semantics and transactional consistency.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::repos::balance_repo::{self, BalanceError};
use crate::services::balance_deltas::{self, JournalLineInput};

/// Update account balances from journal lines within a transaction
///
/// This function is called as part of the posting transaction to ensure
/// exactly-once balance updates. It:
/// 1. Computes balance deltas from journal lines (grouped by account/currency)
/// 2. Upserts each delta into account_balances table
///
/// # Arguments
/// * `tx` - Database transaction (same one used for journal entry creation)
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period identifier (from posting date)
/// * `currency` - Journal entry currency
/// * `journal_entry_id` - Journal entry that triggered this balance update
/// * `lines` - Journal lines to process (account_ref, debit_minor, credit_minor)
///
/// # Returns
/// * `Ok(())` - Balances successfully updated
/// * `Err(BalanceError)` - Database error or validation error
///
/// # Idempotency
/// This function is idempotent when called within the same transaction as
/// journal entry creation. If the posting event is replayed (duplicate event_id),
/// the journal entry won't be created (due to source_event_id dedup), and this
/// function won't be called. Therefore, balances are never double-applied.
///
/// # Example
/// ```ignore
/// let lines = vec![
///     JournalLineInput {
///         account_ref: "1000".to_string(),
///         debit_minor: 10000,
///         credit_minor: 0,
///     },
///     JournalLineInput {
///         account_ref: "4000".to_string(),
///         debit_minor: 0,
///         credit_minor: 10000,
///     },
/// ];
///
/// update_balances_from_journal(
///     &mut tx,
///     "tenant_123",
///     period_id,
///     "USD",
///     entry_id,
///     &lines,
/// ).await?;
/// ```
pub async fn update_balances_from_journal(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,
    journal_entry_id: Uuid,
    lines: &[JournalLineInput],
) -> Result<(), BalanceError> {
    // Compute balance deltas (grouped by account/currency)
    let deltas = balance_deltas::compute_deltas(lines, currency)
        .map_err(|e| BalanceError::InvalidState(format!("Delta computation failed: {}", e)))?;

    let delta_count = deltas.len();

    tracing::debug!(
        journal_entry_id = %journal_entry_id,
        tenant_id = %tenant_id,
        period_id = %period_id,
        delta_count = delta_count,
        "Computed balance deltas from journal lines"
    );

    // Apply each delta to account_balances
    for delta in deltas {
        let balance = balance_repo::tx_upsert_rollup(
            tx,
            tenant_id,
            period_id,
            &delta.account_code,
            &delta.currency,
            delta.debit_delta,
            delta.credit_delta,
            journal_entry_id,
        )
        .await?;

        tracing::debug!(
            balance_id = %balance.id,
            account_code = %delta.account_code,
            currency = %delta.currency,
            debit_delta = delta.debit_delta,
            credit_delta = delta.credit_delta,
            net_balance_minor = balance.net_balance_minor,
            "Updated account balance"
        );
    }

    tracing::info!(
        journal_entry_id = %journal_entry_id,
        tenant_id = %tenant_id,
        period_id = %period_id,
        deltas_applied = delta_count,
        "Successfully updated account balances"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_updater_module_exists() {
        // Basic smoke test to ensure module compiles
        assert!(true);
    }
}
