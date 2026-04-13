//! GL (General Ledger) invariant checks.
//!
//! Invariants:
//! 1. journal_entry_balanced: For every journal entry,
//!    SUM(debit_minor) = SUM(credit_minor).
//! 2. closed_period_hash_present: Every closed accounting period (is_closed=true)
//!    must have close_hash IS NOT NULL. The hash was computed at close time;
//!    its absence indicates the close workflow was bypassed.
//!
//! SQL forms (for manual verification):
//! ```sql
//! -- Invariant 1: journal_entry_balanced
//! SELECT je.id, je.tenant_id,
//!        SUM(jl.debit_minor) AS total_debits,
//!        SUM(jl.credit_minor) AS total_credits
//! FROM journal_entries je
//! JOIN journal_lines jl ON jl.journal_entry_id = je.id
//! GROUP BY je.id, je.tenant_id
//! HAVING SUM(jl.debit_minor) <> SUM(jl.credit_minor);
//!
//! -- Invariant 2: closed_period_hash_present
//! SELECT id, tenant_id, period_start, period_end
//! FROM accounting_periods
//! WHERE is_closed = true AND close_hash IS NULL;
//! ```

use anyhow::Result;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use super::Violation;

const MODULE: &str = "gl";

/// Run all GL invariant checks. Returns list of violations found.
pub async fn run_checks(pool: &PgPool) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();

    info!("GL: checking journal_entry_balanced invariant");
    violations.extend(check_journal_entry_balanced(pool).await?);

    info!("GL: checking closed_period_hash_present invariant");
    violations.extend(check_closed_period_hash_present(pool).await?);

    Ok(violations)
}

/// Invariant 1: All journal entries are balanced (debits == credits).
///
/// Double-entry accounting fundamental: every journal entry must have
/// equal total debit and credit amounts. An unbalanced entry indicates
/// a posting bug or partial write failure.
async fn check_journal_entry_balanced(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT je.id,
               je.tenant_id,
               COALESCE(SUM(jl.debit_minor), 0)::BIGINT AS total_debits,
               COALESCE(SUM(jl.credit_minor), 0)::BIGINT AS total_credits
        FROM journal_entries je
        LEFT JOIN journal_lines jl ON jl.journal_entry_id = je.id
        GROUP BY je.id, je.tenant_id
        HAVING COALESCE(SUM(jl.debit_minor), 0) <> COALESCE(SUM(jl.credit_minor), 0)
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (entry_id, tenant_id, debits, credits) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "journal_entry_balanced",
        rows.len() as i64,
        format!(
            "first violation: entry_id={entry_id} tenant_id={tenant_id} debits={debits} credits={credits} diff={}",
            debits - credits
        ),
    )])
}

/// Invariant 2: Every closed accounting period has a close hash.
///
/// When a period is closed, the close workflow computes a SHA-256 hash over the
/// period's journal entries and stores it in accounting_periods.close_hash.
/// A closed period without a hash indicates the close workflow was bypassed
/// or the hash was cleared after the fact.
async fn check_closed_period_hash_present(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        r#"
        SELECT id, tenant_id
        FROM accounting_periods
        WHERE is_closed = true
          AND close_hash IS NULL
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (period_id, tenant_id) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "closed_period_hash_present",
        rows.len() as i64,
        format!("first violation: period_id={period_id} tenant_id={tenant_id}"),
    )])
}
