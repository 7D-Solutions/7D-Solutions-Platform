//! GL Module Invariant Primitives (Phase 15 - bd-35x)
//!
//! **Purpose:** Module-scoped assertion functions that enforce GL-specific invariants
//! before cross-module E2E testing (bd-3rc).
//!
//! **Invariants Enforced:**
//! 1. Balanced entries (sum debits == sum credits)
//! 2. No duplicate postings (source_event_id UNIQUE)
//! 3. Account validation (account_ref exists and is active)
//! 4. Period validation (no posting into closed periods)
//! 5. Line number uniqueness (UNIQUE per journal_entry_id)
//!
//! **Usage:**
//! ```rust
//! use gl_rs::invariants::*;
//!
//! // Check journal entry balance
//! assert_all_entries_balanced(&pool, tenant_id).await?;
//!
//! // Check no duplicate postings
//! assert_no_duplicate_postings(&pool, tenant_id).await?;
//! ```

use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantViolation {
    /// Journal entry is not balanced (debits != credits)
    UnbalancedEntry {
        entry_id: Uuid,
        total_debits: i64,
        total_credits: i64,
        difference: i64,
    },
    /// Duplicate posting for same source event (source_event_id duplicate)
    DuplicatePosting {
        source_event_id: Uuid,
        count: i64,
    },
    /// Journal line references non-existent or inactive account
    InvalidAccountReference {
        line_id: Uuid,
        account_ref: String,
        reason: String,
    },
    /// Journal entry posted into closed accounting period
    PostingIntoClosedPeriod {
        entry_id: Uuid,
        posted_at: String,
        period_id: Option<Uuid>,
    },
    /// Duplicate line numbers within same journal entry
    DuplicateLineNumbers {
        entry_id: Uuid,
        line_no: i32,
        count: i64,
    },
    /// Database query error
    DatabaseError(String),
}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnbalancedEntry {
                entry_id,
                total_debits,
                total_credits,
                difference,
            } => write!(
                f,
                "Unbalanced entry {}: debits={}, credits={}, difference={}",
                entry_id, total_debits, total_credits, difference
            ),
            Self::DuplicatePosting {
                source_event_id,
                count,
            } => write!(
                f,
                "Duplicate posting for source_event_id {}: count={}",
                source_event_id, count
            ),
            Self::InvalidAccountReference {
                line_id,
                account_ref,
                reason,
            } => write!(
                f,
                "Line {} references invalid account '{}': {}",
                line_id, account_ref, reason
            ),
            Self::PostingIntoClosedPeriod {
                entry_id,
                posted_at,
                period_id,
            } => write!(
                f,
                "Entry {} posted at {} into closed period {:?}",
                entry_id, posted_at, period_id
            ),
            Self::DuplicateLineNumbers {
                entry_id,
                line_no,
                count,
            } => write!(
                f,
                "Entry {} has duplicate line_no {}: count={}",
                entry_id, line_no, count
            ),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for InvariantViolation {}

impl From<sqlx::Error> for InvariantViolation {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Invariant Assertion Functions
// ============================================================================

/// Assert: All journal entries are balanced (debits == credits)
///
/// **Invariant:** Double-entry accounting fundamental - every entry must balance
///
/// **Checks:**
/// - For each journal_entry_id, SUM(debit_minor) == SUM(credit_minor)
/// - Accounting equation integrity
/// - No unbalanced postings
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_all_entries_balanced(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for unbalanced entries
    let unbalanced: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT je.id,
                COALESCE(SUM(jl.debit_minor), 0) as total_debits,
                COALESCE(SUM(jl.credit_minor), 0) as total_credits
         FROM journal_entries je
         LEFT JOIN journal_lines jl ON jl.journal_entry_id = je.id
         WHERE je.tenant_id = $1
         GROUP BY je.id
         HAVING COALESCE(SUM(jl.debit_minor), 0) != COALESCE(SUM(jl.credit_minor), 0)"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((entry_id, total_debits, total_credits)) = unbalanced.first() {
        let difference = total_debits - total_credits;
        return Err(InvariantViolation::UnbalancedEntry {
            entry_id: *entry_id,
            total_debits: *total_debits,
            total_credits: *total_credits,
            difference,
        });
    }

    Ok(())
}

/// Assert: No duplicate postings (source_event_id uniqueness)
///
/// **Invariant:** UNIQUE(source_event_id) constraint integrity
///
/// **Checks:**
/// - Each source_event_id appears at most once in journal_entries
/// - Idempotency preserved
/// - No duplicate event processing
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_duplicate_postings(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for duplicate source_event_ids
    let duplicates: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT source_event_id, COUNT(*) as count
         FROM journal_entries
         WHERE tenant_id = $1
         GROUP BY source_event_id
         HAVING COUNT(*) > 1"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((source_event_id, count)) = duplicates.first() {
        return Err(InvariantViolation::DuplicatePosting {
            source_event_id: *source_event_id,
            count: *count,
        });
    }

    Ok(())
}

/// Assert: All journal lines reference valid accounts
///
/// **Invariant:** Foreign key integrity - account_ref must exist and be active
///
/// **Checks:**
/// - Each journal_line.account_ref exists in accounts table
/// - Referenced account is_active = true
/// - Chart of accounts integrity (Phase 10)
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_valid_account_references(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for journal lines with invalid account references
    let invalid: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT jl.id, jl.account_ref
         FROM journal_lines jl
         JOIN journal_entries je ON je.id = jl.journal_entry_id
         LEFT JOIN accounts a ON a.tenant_id = je.tenant_id AND a.code = jl.account_ref
         WHERE je.tenant_id = $1
           AND (a.id IS NULL OR a.is_active = false)"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((line_id, account_ref)) = invalid.first() {
        return Err(InvariantViolation::InvalidAccountReference {
            line_id: *line_id,
            account_ref: account_ref.clone(),
            reason: "Account does not exist or is inactive".to_string(),
        });
    }

    Ok(())
}

/// Assert: No postings into closed accounting periods
///
/// **Invariant:** Closed periods are immutable - no new postings allowed
///
/// **Checks:**
/// - Each journal_entry.posted_at falls outside closed period boundaries
/// - Period close enforcement (Phase 13)
/// - Audit-grade immutability
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_closed_period_postings(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for entries posted into closed periods
    let violations: Vec<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT je.id, je.posted_at::text, ap.id
         FROM journal_entries je
         JOIN accounting_periods ap ON ap.tenant_id = je.tenant_id
         WHERE je.tenant_id = $1
           AND je.posted_at::date >= ap.period_start
           AND je.posted_at::date <= ap.period_end
           AND ap.is_closed = true"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((entry_id, posted_at, period_id)) = violations.first() {
        return Err(InvariantViolation::PostingIntoClosedPeriod {
            entry_id: *entry_id,
            posted_at: posted_at.clone(),
            period_id: *period_id,
        });
    }

    Ok(())
}

/// Assert: No duplicate line numbers within journal entries
///
/// **Invariant:** UNIQUE(journal_entry_id, line_no) constraint integrity
///
/// **Checks:**
/// - Each line_no appears at most once per journal_entry_id
/// - Line numbering integrity
/// - Prevents line collision
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_unique_line_numbers(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for duplicate line numbers
    let duplicates: Vec<(Uuid, i32, i64)> = sqlx::query_as(
        "SELECT jl.journal_entry_id, jl.line_no, COUNT(*) as count
         FROM journal_lines jl
         JOIN journal_entries je ON je.id = jl.journal_entry_id
         WHERE je.tenant_id = $1
         GROUP BY jl.journal_entry_id, jl.line_no
         HAVING COUNT(*) > 1"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((entry_id, line_no, count)) = duplicates.first() {
        return Err(InvariantViolation::DuplicateLineNumbers {
            entry_id: *entry_id,
            line_no: *line_no,
            count: *count,
        });
    }

    Ok(())
}

/// Run all GL invariant checks
///
/// **Convenience function** that runs all GL-specific invariant assertions.
///
/// **Returns:** Ok(()) if all invariants hold, Err on first violation
pub async fn assert_all_invariants(pool: &PgPool, tenant_id: &str) -> Result<(), InvariantViolation> {
    assert_all_entries_balanced(pool, tenant_id).await?;
    assert_no_duplicate_postings(pool, tenant_id).await?;
    assert_valid_account_references(pool, tenant_id).await?;
    assert_no_closed_period_postings(pool, tenant_id).await?;
    assert_unique_line_numbers(pool, tenant_id).await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invariant_violation_display() {
        let entry_id = Uuid::nil();

        let violation = InvariantViolation::UnbalancedEntry {
            entry_id,
            total_debits: 10000,
            total_credits: 9000,
            difference: 1000,
        };
        assert!(violation.to_string().contains("Unbalanced entry"));
        assert!(violation.to_string().contains("difference=1000"));

        let violation = InvariantViolation::DuplicatePosting {
            source_event_id: Uuid::nil(),
            count: 2,
        };
        assert!(violation.to_string().contains("Duplicate posting"));
        assert!(violation.to_string().contains("count=2"));

        let violation = InvariantViolation::InvalidAccountReference {
            line_id: Uuid::nil(),
            account_ref: "ACC-123".to_string(),
            reason: "Account does not exist".to_string(),
        };
        assert!(violation.to_string().contains("invalid account"));
        assert!(violation.to_string().contains("ACC-123"));
    }

    #[test]
    fn test_invariant_violation_from_sqlx_error() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let violation: InvariantViolation = sqlx_err.into();
        assert!(matches!(violation, InvariantViolation::DatabaseError(_)));
    }
}
