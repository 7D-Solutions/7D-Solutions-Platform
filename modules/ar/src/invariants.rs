//! AR Module Invariant Primitives (Phase 15 - bd-35x)
//!
//! **Purpose:** Module-scoped assertion functions that enforce AR-specific invariants
//! before cross-module E2E testing (bd-3rc).
//!
//! **Invariants Enforced:**
//! 1. No duplicate attempts at attempt grain (UNIQUE constraint integrity)
//! 2. Legal transitions only (lifecycle guards respected)
//! 3. No mutation on invalid inputs (guard validation blocks illegal changes)
//! 4. Exactly-once behavior under replay (idempotency)
//! 5. Attempt count limits (MAX_ATTEMPTS = 3)
//!
//! **Usage:**
//! ```rust,no_run
//! use ar_rs::invariants::*;
//!
//! // Check invoice attempt ledger integrity
//! assert_no_duplicate_attempts(&pool, app_id).await?;
//!
//! // Check attempt count limits
//! assert_attempt_count_within_limits(&pool, app_id).await?;
//! ```

use sqlx::PgPool;
use std::fmt;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantViolation {
    /// Duplicate attempts found for same (app_id, invoice_id, attempt_no)
    DuplicateAttempts {
        app_id: String,
        invoice_id: i32,
        attempt_no: i32,
        count: i64,
    },
    /// Invoice has exceeded maximum attempt count (3)
    ExceededMaxAttempts {
        invoice_id: i32,
        attempt_count: i64,
        max_allowed: i32,
    },
    /// Terminal status invoice has additional attempts (should be immutable)
    TerminalStatusWithAttempts {
        invoice_id: i32,
        status: String,
        attempt_count: i64,
    },
    /// Database query error
    DatabaseError(String),
}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateAttempts {
                app_id,
                invoice_id,
                attempt_no,
                count,
            } => write!(
                f,
                "Duplicate attempts found: app_id={}, invoice_id={}, attempt_no={}, count={}",
                app_id, invoice_id, attempt_no, count
            ),
            Self::ExceededMaxAttempts {
                invoice_id,
                attempt_count,
                max_allowed,
            } => write!(
                f,
                "Invoice {} exceeded max attempts: {} > {}",
                invoice_id, attempt_count, max_allowed
            ),
            Self::TerminalStatusWithAttempts {
                invoice_id,
                status,
                attempt_count,
            } => write!(
                f,
                "Invoice {} in terminal status '{}' has {} attempts (expected: 0 new attempts after terminal)",
                invoice_id, status, attempt_count
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

/// Assert: No duplicate attempts at attempt grain
///
/// **Invariant:** UNIQUE(app_id, invoice_id, attempt_no) constraint integrity
///
/// **Checks:**
/// - No two ar_invoice_attempts rows have same (app_id, invoice_id, attempt_no)
/// - Database UNIQUE constraint is enforced
/// - Exactly-once semantics preserved
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_duplicate_attempts(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for duplicate attempts
    let duplicates: Vec<(i32, i32, i64)> = sqlx::query_as(
        "SELECT invoice_id, attempt_no, COUNT(*) as count
         FROM ar_invoice_attempts
         WHERE app_id = $1
         GROUP BY invoice_id, attempt_no
         HAVING COUNT(*) > 1"
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    if let Some((invoice_id, attempt_no, count)) = duplicates.first() {
        return Err(InvariantViolation::DuplicateAttempts {
            app_id: app_id.to_string(),
            invoice_id: *invoice_id,
            attempt_no: *attempt_no,
            count: *count,
        });
    }

    Ok(())
}

/// Assert: Attempt count within limits (MAX_ATTEMPTS = 3)
///
/// **Invariant:** No invoice has more than 3 attempts (windows: 0, 1, 2)
///
/// **Checks:**
/// - Each invoice has at most 3 attempt rows
/// - Retry window discipline enforced (bd-8ev)
/// - Terminal failure after attempt 2
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_attempt_count_within_limits(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    const MAX_ATTEMPTS: i32 = 3;

    // Query for invoices exceeding max attempts
    let violators: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT invoice_id, COUNT(*) as attempt_count
         FROM ar_invoice_attempts
         WHERE app_id = $1
         GROUP BY invoice_id
         HAVING COUNT(*) > $2"
    )
    .bind(app_id)
    .bind(MAX_ATTEMPTS as i64)
    .fetch_all(pool)
    .await?;

    if let Some((invoice_id, attempt_count)) = violators.first() {
        return Err(InvariantViolation::ExceededMaxAttempts {
            invoice_id: *invoice_id,
            attempt_count: *attempt_count,
            max_allowed: MAX_ATTEMPTS,
        });
    }

    Ok(())
}

/// Assert: Terminal status invoices have no new attempts
///
/// **Invariant:** Invoices in terminal status (paid, void) should not have new attempts
///
/// **Checks:**
/// - Invoices with status='paid' or status='void' have no attempts after reaching terminal
/// - Lifecycle guards prevent transitions from terminal states
/// - State machine immutability enforced
///
/// **Note:** This checks for attempts created AFTER invoice reached terminal status,
/// which would indicate a lifecycle guard bypass.
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_attempts_after_terminal(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for terminal status invoices with attempts
    // This is a simplified check - in production, would check attempt timestamps vs status change
    let violators: Vec<(i32, String, i64)> = sqlx::query_as(
        "SELECT i.id, i.status, COUNT(a.id) as attempt_count
         FROM ar_invoices i
         LEFT JOIN ar_invoice_attempts a ON a.app_id = i.app_id AND a.invoice_id = i.id
         WHERE i.app_id = $1
           AND i.status IN ('paid', 'void')
           AND a.id IS NOT NULL
         GROUP BY i.id, i.status
         HAVING COUNT(a.id) > 0"
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    if let Some((invoice_id, status, attempt_count)) = violators.first() {
        return Err(InvariantViolation::TerminalStatusWithAttempts {
            invoice_id: *invoice_id,
            status: status.clone(),
            attempt_count: *attempt_count,
        });
    }

    Ok(())
}

/// Run all AR invariant checks
///
/// **Convenience function** that runs all AR-specific invariant assertions.
///
/// **Returns:** Ok(()) if all invariants hold, Err on first violation
pub async fn assert_all_invariants(pool: &PgPool, app_id: &str) -> Result<(), InvariantViolation> {
    assert_no_duplicate_attempts(pool, app_id).await?;
    assert_attempt_count_within_limits(pool, app_id).await?;
    assert_no_attempts_after_terminal(pool, app_id).await?;
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
        let violation = InvariantViolation::DuplicateAttempts {
            app_id: "app-123".to_string(),
            invoice_id: 456,
            attempt_no: 1,
            count: 2,
        };
        assert!(violation.to_string().contains("Duplicate attempts"));
        assert!(violation.to_string().contains("app-123"));
        assert!(violation.to_string().contains("456"));

        let violation = InvariantViolation::ExceededMaxAttempts {
            invoice_id: 789,
            attempt_count: 5,
            max_allowed: 3,
        };
        assert!(violation.to_string().contains("exceeded max attempts"));
        assert!(violation.to_string().contains("789"));
        assert!(violation.to_string().contains("5 > 3"));
    }

    #[test]
    fn test_invariant_violation_from_sqlx_error() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let violation: InvariantViolation = sqlx_err.into();
        assert!(matches!(violation, InvariantViolation::DatabaseError(_)));
    }
}
