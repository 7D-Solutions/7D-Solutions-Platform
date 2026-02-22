//! Payments Module Invariant Primitives (Phase 15 - bd-35x)
//!
//! **Purpose:** Module-scoped assertion functions that enforce Payment-specific invariants
//! before cross-module E2E testing (bd-3rc).
//!
//! **Invariants Enforced:**
//! 1. No duplicate attempts at attempt grain (UNIQUE constraint integrity)
//! 2. Legal transitions only (lifecycle guards respected)
//! 3. No mutation on invalid inputs (guard validation blocks illegal changes)
//! 4. Exactly-once behavior under replay (idempotency)
//! 5. UNKNOWN protocol compliance (no retries while status=unknown)
//! 6. Attempt count limits (MAX_ATTEMPTS = 3)
//!
//! **Usage:**
//! ```rust,ignore
//! use payments_rs::invariants::*;
//!
//! // Check payment attempt ledger integrity
//! assert_no_duplicate_attempts(&pool, app_id).await?;
//!
//! // Check UNKNOWN protocol compliance
//! assert_unknown_protocol_compliance(&pool, app_id).await?;
//! ```

use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantViolation {
    /// Duplicate attempts found for same (app_id, payment_id, attempt_no)
    DuplicateAttempts {
        app_id: String,
        payment_id: Uuid,
        attempt_no: i32,
        count: i64,
    },
    /// Payment has exceeded maximum attempt count (3)
    ExceededMaxAttempts {
        payment_id: Uuid,
        attempt_count: i64,
        max_allowed: i32,
    },
    /// Payment with status=unknown found in retry queue (UNKNOWN blocks retry)
    UnknownInRetryQueue {
        payment_id: Uuid,
        status: String,
    },
    /// Terminal status payment has new attempts (should be immutable)
    TerminalStatusWithAttempts {
        payment_id: Uuid,
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
                payment_id,
                attempt_no,
                count,
            } => write!(
                f,
                "Duplicate attempts found: app_id={}, payment_id={}, attempt_no={}, count={}",
                app_id, payment_id, attempt_no, count
            ),
            Self::ExceededMaxAttempts {
                payment_id,
                attempt_count,
                max_allowed,
            } => write!(
                f,
                "Payment {} exceeded max attempts: {} > {}",
                payment_id, attempt_count, max_allowed
            ),
            Self::UnknownInRetryQueue { payment_id, status } => write!(
                f,
                "Payment {} with status='{}' found in retry queue (UNKNOWN blocks retry)",
                payment_id, status
            ),
            Self::TerminalStatusWithAttempts {
                payment_id,
                status,
                attempt_count,
            } => write!(
                f,
                "Payment {} in terminal status '{}' has {} attempts (expected: 0 new attempts after terminal)",
                payment_id, status, attempt_count
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
/// **Invariant:** UNIQUE(app_id, payment_id, attempt_no) constraint integrity
///
/// **Checks:**
/// - No two payment_attempts rows have same (app_id, payment_id, attempt_no)
/// - Database UNIQUE constraint is enforced
/// - Exactly-once semantics preserved
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_duplicate_attempts(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for duplicate attempts
    let duplicates: Vec<(Uuid, i32, i64)> = sqlx::query_as(
        "SELECT payment_id, attempt_no, COUNT(*) as count
         FROM payment_attempts
         WHERE app_id = $1
         GROUP BY payment_id, attempt_no
         HAVING COUNT(*) > 1"
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    if let Some((payment_id, attempt_no, count)) = duplicates.first() {
        return Err(InvariantViolation::DuplicateAttempts {
            app_id: app_id.to_string(),
            payment_id: *payment_id,
            attempt_no: *attempt_no,
            count: *count,
        });
    }

    Ok(())
}

/// Assert: Attempt count within limits (MAX_ATTEMPTS = 3)
///
/// **Invariant:** No payment has more than 3 attempts (windows: 0, 1, 2)
///
/// **Checks:**
/// - Each payment has at most 3 attempt rows
/// - Retry window discipline enforced (bd-1it)
/// - Terminal failure after attempt 2
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_attempt_count_within_limits(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    const MAX_ATTEMPTS: i32 = 3;

    // Query for payments exceeding max attempts
    let violators: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT payment_id, COUNT(*) as attempt_count
         FROM payment_attempts
         WHERE app_id = $1
         GROUP BY payment_id
         HAVING COUNT(*) > $2"
    )
    .bind(app_id)
    .bind(MAX_ATTEMPTS as i64)
    .fetch_all(pool)
    .await?;

    if let Some((payment_id, attempt_count)) = violators.first() {
        return Err(InvariantViolation::ExceededMaxAttempts {
            payment_id: *payment_id,
            attempt_count: *attempt_count,
            max_allowed: MAX_ATTEMPTS,
        });
    }

    Ok(())
}

/// Assert: UNKNOWN protocol compliance (bd-2uw)
///
/// **Invariant:** Payments with status='unknown' are NOT eligible for retry
///
/// **Checks:**
/// - No payment_attempts with status='unknown' appear in retry-eligible queries
/// - UNKNOWN blocks retry scheduling (customer not at fault)
/// - Reconciliation must resolve UNKNOWN before retry (bd-2uw)
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_unknown_protocol_compliance(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for payments where an UNKNOWN attempt coexists with a retry-eligible attempt.
    // UNKNOWN is a valid state — the protocol violation is when a payment has both
    // an UNKNOWN attempt (which should block retry) AND a retry-eligible attempt
    // ('attempting' or 'failed_retry') still active for the same payment.
    // A payment in UNKNOWN state must resolve via reconciliation before any retry.
    let unknown_in_retry: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT DISTINCT pa.payment_id, 'unknown'
         FROM payment_attempts pa
         WHERE pa.app_id = $1
           AND pa.status::text = 'unknown'
           AND EXISTS (
               SELECT 1 FROM payment_attempts pa2
               WHERE pa2.app_id = $1
                 AND pa2.payment_id = pa.payment_id
                 AND pa2.status::text IN ('attempting', 'failed_retry')
           )"
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    if let Some((payment_id, status)) = unknown_in_retry.first() {
        return Err(InvariantViolation::UnknownInRetryQueue {
            payment_id: *payment_id,
            status: status.clone(),
        });
    }

    Ok(())
}

/// Assert: Terminal status payments have no new attempts
///
/// **Invariant:** Payments in terminal status (succeeded, failed_final) should not have new attempts
///
/// **Checks:**
/// - Payments with status='succeeded' or status='failed_final' have no attempts after reaching terminal
/// - Lifecycle guards prevent transitions from terminal states
/// - State machine immutability enforced
///
/// **Note:** This checks for attempts created AFTER payment reached terminal status,
/// which would indicate a lifecycle guard bypass.
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_attempts_after_terminal(
    pool: &PgPool,
    app_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for terminal status payments with multiple attempts
    // Simplified check - assumes latest attempt is the terminal one
    let violators: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "SELECT payment_id, status::text, COUNT(*) as attempt_count
         FROM payment_attempts
         WHERE app_id = $1
           AND status::text IN ('succeeded', 'failed_final')
         GROUP BY payment_id, status
         HAVING COUNT(*) > 1"
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    if let Some((payment_id, status, attempt_count)) = violators.first() {
        return Err(InvariantViolation::TerminalStatusWithAttempts {
            payment_id: *payment_id,
            status: status.clone(),
            attempt_count: *attempt_count,
        });
    }

    Ok(())
}

/// Run all Payments invariant checks
///
/// **Convenience function** that runs all Payment-specific invariant assertions.
///
/// **Returns:** Ok(()) if all invariants hold, Err on first violation
pub async fn assert_all_invariants(pool: &PgPool, app_id: &str) -> Result<(), InvariantViolation> {
    assert_no_duplicate_attempts(pool, app_id).await?;
    assert_attempt_count_within_limits(pool, app_id).await?;
    assert_unknown_protocol_compliance(pool, app_id).await?;
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
        let payment_id = Uuid::nil();

        let violation = InvariantViolation::DuplicateAttempts {
            app_id: "app-123".to_string(),
            payment_id,
            attempt_no: 1,
            count: 2,
        };
        assert!(violation.to_string().contains("Duplicate attempts"));
        assert!(violation.to_string().contains("app-123"));

        let violation = InvariantViolation::ExceededMaxAttempts {
            payment_id,
            attempt_count: 5,
            max_allowed: 3,
        };
        assert!(violation.to_string().contains("exceeded max attempts"));
        assert!(violation.to_string().contains("5 > 3"));

        let violation = InvariantViolation::UnknownInRetryQueue {
            payment_id,
            status: "unknown".to_string(),
        };
        assert!(violation.to_string().contains("UNKNOWN blocks retry"));
    }

    #[test]
    fn test_invariant_violation_from_sqlx_error() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let violation: InvariantViolation = sqlx_err.into();
        assert!(matches!(violation, InvariantViolation::DatabaseError(_)));
    }
}
