//! Period Close Snapshot
//!
//! Snapshot computation and sealing for accounting period closes.
//! Provides deterministic hash computation for audit trail integrity.

use super::period_close_validation::PeriodCloseError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Period close snapshot - sealed data for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodCloseSnapshot {
    pub period_id: Uuid,
    pub tenant_id: String,
    pub close_hash: String,
    pub total_journal_count: i64,
    pub total_debits_minor: i64,
    pub total_credits_minor: i64,
    pub balance_row_count: i64,
    pub currency_snapshots: Vec<CurrencySnapshot>,
}

/// Per-currency snapshot data
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CurrencySnapshot {
    pub currency: String,
    pub journal_count: i32,
    pub line_count: i32,
    pub total_debits_minor: i64,
    pub total_credits_minor: i64,
}

/// Compute period summary snapshots for all currencies in a period
///
/// This queries journal_lines to get accurate counts (journal_count, line_count)
/// and totals (debits, credits) for each currency.
///
/// # Arguments
/// * `tx` - Database transaction (ensures atomicity with close operation)
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
///
/// # Returns
/// Vector of currency snapshots with accurate counts and totals
async fn compute_currency_snapshots(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Vec<CurrencySnapshot>, PeriodCloseError> {
    // Query journal_lines grouped by currency to get accurate counts and totals
    // Note: This is the one place where we DO scan journal_lines (at close time only)
    // Currency is on journal_entries table, not journal_lines
    // Period is determined by joining with accounting_periods on date range
    let snapshots = sqlx::query_as::<_, CurrencySnapshot>(
        r#"
        SELECT
            je.currency,
            COUNT(DISTINCT je.id)::INTEGER as journal_count,
            COUNT(jl.id)::INTEGER as line_count,
            COALESCE(SUM(jl.debit_minor), 0)::BIGINT as total_debits_minor,
            COALESCE(SUM(jl.credit_minor), 0)::BIGINT as total_credits_minor
        FROM accounting_periods ap
        INNER JOIN journal_entries je ON
            je.tenant_id = ap.tenant_id
            AND je.posted_at::DATE >= ap.period_start
            AND je.posted_at::DATE <= ap.period_end
        LEFT JOIN journal_lines jl ON jl.journal_entry_id = je.id
        WHERE ap.id = $1
          AND ap.tenant_id = $2
        GROUP BY je.currency
        ORDER BY je.currency
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_all(&mut **tx)
    .await?;

    Ok(snapshots)
}

/// Compute the balance row count for a period (tenant + period scoped)
///
/// Counts the number of account_balances rows for this tenant and period.
///
/// # Arguments
/// * `tx` - Database transaction
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
///
/// # Returns
/// Count of balance rows
async fn compute_balance_row_count(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<i64, PeriodCloseError> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM account_balances
        WHERE tenant_id = $1
          AND period_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count)
}

/// Compute deterministic close hash from period summary data
///
/// Hash inputs (in order):
/// 1. tenant_id
/// 2. period_id (as string)
/// 3. total_journal_count (sum across all currencies)
/// 4. total_debits_minor (sum across all currencies)
/// 5. total_credits_minor (sum across all currencies)
/// 6. balance_row_count
///
/// Format: SHA-256(tenant_id|period_id|journal_count|debits|credits|balance_count)
///
/// # Arguments
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `total_journal_count` - Total journal entries across all currencies
/// * `total_debits_minor` - Total debits across all currencies
/// * `total_credits_minor` - Total credits across all currencies
/// * `balance_row_count` - Count of account_balances rows
///
/// # Returns
/// Hex-encoded SHA-256 hash (64 characters)
pub fn compute_close_hash(
    tenant_id: &str,
    period_id: Uuid,
    total_journal_count: i64,
    total_debits_minor: i64,
    total_credits_minor: i64,
    balance_row_count: i64,
) -> String {
    let mut hasher = Sha256::new();

    // Hash inputs in deterministic order
    hasher.update(tenant_id.as_bytes());
    hasher.update(b"|");
    hasher.update(period_id.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(total_journal_count.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(total_debits_minor.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(total_credits_minor.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(balance_row_count.to_string().as_bytes());

    // Return hex-encoded hash
    format!("{:x}", hasher.finalize())
}

/// Persist currency snapshots to period_summary_snapshots table
///
/// Inserts snapshots for each currency. Uses ON CONFLICT DO NOTHING to ensure
/// idempotency - sealed snapshots are never overwritten.
///
/// # Arguments
/// * `tx` - Database transaction
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `snapshots` - Currency snapshots to persist
async fn persist_currency_snapshots(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
    snapshots: &[CurrencySnapshot],
) -> Result<(), PeriodCloseError> {
    for snapshot in snapshots {
        sqlx::query(
            r#"
            INSERT INTO period_summary_snapshots (
                tenant_id,
                period_id,
                currency,
                journal_count,
                line_count,
                total_debits_minor,
                total_credits_minor,
                created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            ON CONFLICT (tenant_id, period_id, currency)
            DO NOTHING
            "#,
        )
        .bind(tenant_id)
        .bind(period_id)
        .bind(&snapshot.currency)
        .bind(snapshot.journal_count)
        .bind(snapshot.line_count)
        .bind(snapshot.total_debits_minor)
        .bind(snapshot.total_credits_minor)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// Create a sealed snapshot for period close
///
/// This is called during period close to:
/// 1. Compute accurate snapshots from journal_lines (all currencies)
/// 2. Compute deterministic close hash
/// 3. Persist snapshots atomically in the same transaction
///
/// The snapshot provides audit trail integrity and tamper detection.
///
/// # Arguments
/// * `tx` - Database transaction (must be the same transaction as period close)
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
///
/// # Returns
/// PeriodCloseSnapshot with close hash and currency snapshots
pub async fn create_close_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<PeriodCloseSnapshot, PeriodCloseError> {
    // Step 1: Compute currency snapshots from journal_lines
    let currency_snapshots = compute_currency_snapshots(tx, tenant_id, period_id).await?;

    // Step 2: Compute balance row count
    let balance_row_count = compute_balance_row_count(tx, tenant_id, period_id).await?;

    // Step 3: Aggregate totals across all currencies
    let total_journal_count = currency_snapshots
        .iter()
        .map(|s| s.journal_count as i64)
        .sum();

    let total_debits_minor = currency_snapshots
        .iter()
        .map(|s| s.total_debits_minor)
        .sum();

    let total_credits_minor = currency_snapshots
        .iter()
        .map(|s| s.total_credits_minor)
        .sum();

    // Step 4: Compute deterministic close hash
    let close_hash = compute_close_hash(
        tenant_id,
        period_id,
        total_journal_count,
        total_debits_minor,
        total_credits_minor,
        balance_row_count,
    );

    // Step 5: Persist currency snapshots
    persist_currency_snapshots(tx, tenant_id, period_id, &currency_snapshots).await?;

    Ok(PeriodCloseSnapshot {
        period_id,
        tenant_id: tenant_id.to_string(),
        close_hash,
        total_journal_count,
        total_debits_minor,
        total_credits_minor,
        balance_row_count,
        currency_snapshots,
    })
}

/// Verify that a close hash matches the current period state
///
/// This recomputes the hash from current data and compares it to the expected hash.
/// Used for audit verification and tamper detection.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `expected_hash` - Hash to verify against
///
/// # Returns
/// Ok(()) if hash matches, Err if mismatch or error
pub async fn verify_close_hash(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    expected_hash: &str,
) -> Result<(), PeriodCloseError> {
    // Start a read-only transaction for consistency
    let mut tx = pool.begin().await?;

    // Compute currency snapshots
    let currency_snapshots = compute_currency_snapshots(&mut tx, tenant_id, period_id).await?;

    // Compute balance row count
    let balance_row_count = compute_balance_row_count(&mut tx, tenant_id, period_id).await?;

    // Aggregate totals
    let total_journal_count = currency_snapshots
        .iter()
        .map(|s| s.journal_count as i64)
        .sum();

    let total_debits_minor = currency_snapshots
        .iter()
        .map(|s| s.total_debits_minor)
        .sum();

    let total_credits_minor = currency_snapshots
        .iter()
        .map(|s| s.total_credits_minor)
        .sum();

    // Compute hash
    let computed_hash = compute_close_hash(
        tenant_id,
        period_id,
        total_journal_count,
        total_debits_minor,
        total_credits_minor,
        balance_row_count,
    );

    // Commit read-only transaction
    tx.commit().await?;

    // Compare hashes
    if computed_hash != expected_hash {
        return Err(PeriodCloseError::HashMismatch {
            computed: computed_hash,
            expected: expected_hash.to_string(),
        });
    }

    Ok(())
}
