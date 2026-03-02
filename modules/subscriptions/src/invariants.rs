//! Subscriptions Module Invariant Primitives (Phase 15 - bd-35x)
//!
//! **Purpose:** Module-scoped assertion functions that enforce Subscription-specific invariants
//! before cross-module E2E testing (bd-3rc).
//!
//! **Invariants Enforced:**
//! 1. No duplicate attempts at attempt grain (UNIQUE constraint integrity)
//! 2. Exactly-once per cycle (one invoice per subscription cycle)
//! 3. Legal transitions only (lifecycle guards respected)
//! 4. No mutation on invalid inputs (guard validation blocks illegal changes)
//!
//! **Usage:**
//! ```rust
//! use subscriptions_rs::invariants::*;
//!
//! // Check subscription cycle attempt ledger integrity
//! assert_no_duplicate_cycle_attempts(&pool, tenant_id).await?;
//!
//! // Check exactly-once per cycle
//! assert_one_invoice_per_cycle(&pool, tenant_id).await?;
//! ```

use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantViolation {
    /// Duplicate cycle attempts found for same (tenant_id, subscription_id, cycle_key)
    DuplicateCycleAttempts {
        tenant_id: String,
        subscription_id: Uuid,
        cycle_key: String,
        count: i64,
    },
    /// Multiple invoices generated for same subscription cycle (exactly-once violation)
    MultipleCycleInvoices {
        tenant_id: String,
        subscription_id: Uuid,
        cycle_key: String,
        invoice_count: i64,
    },
    /// Terminal status attempt has additional attempts (should be immutable)
    TerminalStatusWithAttempts {
        subscription_id: Uuid,
        cycle_key: String,
        status: String,
        attempt_count: i64,
    },
    /// Cycle attempt with succeeded status but no AR invoice ID
    SucceededWithoutInvoice { attempt_id: Uuid, cycle_key: String },
    /// Database query error
    DatabaseError(String),
}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCycleAttempts {
                tenant_id,
                subscription_id,
                cycle_key,
                count,
            } => write!(
                f,
                "Duplicate cycle attempts found: tenant_id={}, subscription_id={}, cycle_key={}, count={}",
                tenant_id, subscription_id, cycle_key, count
            ),
            Self::MultipleCycleInvoices {
                tenant_id,
                subscription_id,
                cycle_key,
                invoice_count,
            } => write!(
                f,
                "Multiple invoices for cycle: tenant_id={}, subscription_id={}, cycle_key={}, invoice_count={}",
                tenant_id, subscription_id, cycle_key, invoice_count
            ),
            Self::TerminalStatusWithAttempts {
                subscription_id,
                cycle_key,
                status,
                attempt_count,
            } => write!(
                f,
                "Subscription {} cycle {} in terminal status '{}' has {} attempts (expected: 1)",
                subscription_id, cycle_key, status, attempt_count
            ),
            Self::SucceededWithoutInvoice {
                attempt_id,
                cycle_key,
            } => write!(
                f,
                "Attempt {} for cycle {} marked 'succeeded' but has no ar_invoice_id",
                attempt_id, cycle_key
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

/// Assert: No duplicate cycle attempts at attempt grain
///
/// **Invariant:** UNIQUE(tenant_id, subscription_id, cycle_key) constraint integrity
///
/// **Checks:**
/// - No two subscription_invoice_attempts rows have same (tenant_id, subscription_id, cycle_key)
/// - Database UNIQUE constraint is enforced
/// - Exactly-once semantics preserved
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_duplicate_cycle_attempts(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for duplicate cycle attempts
    let duplicates: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "SELECT subscription_id, cycle_key, COUNT(*) as count
         FROM subscription_invoice_attempts
         WHERE tenant_id = $1
         GROUP BY subscription_id, cycle_key
         HAVING COUNT(*) > 1",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((subscription_id, cycle_key, count)) = duplicates.first() {
        return Err(InvariantViolation::DuplicateCycleAttempts {
            tenant_id: tenant_id.to_string(),
            subscription_id: *subscription_id,
            cycle_key: cycle_key.clone(),
            count: *count,
        });
    }

    Ok(())
}

/// Assert: Exactly one invoice per subscription cycle
///
/// **Invariant:** No subscription cycle generates multiple invoices (exactly-once)
///
/// **Checks:**
/// - Each (subscription_id, cycle_key) pair has at most one succeeded attempt
/// - Cycle gating (bd-184) prevents duplicate invoice generation
/// - Financial integrity preserved
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_one_invoice_per_cycle(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for cycles with multiple succeeded attempts
    let violators: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "SELECT subscription_id, cycle_key, COUNT(*) as invoice_count
         FROM subscription_invoice_attempts
         WHERE tenant_id = $1
           AND status = 'succeeded'
         GROUP BY subscription_id, cycle_key
         HAVING COUNT(*) > 1",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((subscription_id, cycle_key, invoice_count)) = violators.first() {
        return Err(InvariantViolation::MultipleCycleInvoices {
            tenant_id: tenant_id.to_string(),
            subscription_id: *subscription_id,
            cycle_key: cycle_key.clone(),
            invoice_count: *invoice_count,
        });
    }

    Ok(())
}

/// Assert: Terminal status attempts have no additional attempts
///
/// **Invariant:** Attempts in terminal status (succeeded, failed_final) should not have duplicates
///
/// **Checks:**
/// - Each cycle has at most one attempt
/// - Terminal attempts are immutable
/// - Lifecycle guards prevent re-attempts after terminal state
///
/// **Note:** This checks that terminal status cycles have exactly 1 attempt row,
/// which would indicate no duplicate attempts were created.
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_no_attempts_after_terminal(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for terminal status cycles with multiple attempts
    let violators: Vec<(Uuid, String, String, i64)> = sqlx::query_as(
        "SELECT subscription_id, cycle_key, status, COUNT(*) as attempt_count
         FROM subscription_invoice_attempts
         WHERE tenant_id = $1
           AND status IN ('succeeded', 'failed_final')
         GROUP BY subscription_id, cycle_key, status
         HAVING COUNT(*) > 1",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((subscription_id, cycle_key, status, attempt_count)) = violators.first() {
        return Err(InvariantViolation::TerminalStatusWithAttempts {
            subscription_id: *subscription_id,
            cycle_key: cycle_key.clone(),
            status: status.clone(),
            attempt_count: *attempt_count,
        });
    }

    Ok(())
}

/// Assert: Succeeded attempts have AR invoice ID
///
/// **Invariant:** Attempts with status='succeeded' must have ar_invoice_id set
///
/// **Checks:**
/// - Succeeded attempts link to created AR invoices
/// - Data integrity between subscription attempts and AR invoices
/// - No orphaned success status
///
/// **Returns:** Ok(()) if invariant holds, Err(InvariantViolation) otherwise
pub async fn assert_succeeded_has_invoice_id(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    // Query for succeeded attempts without AR invoice ID
    let orphaned: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, cycle_key
         FROM subscription_invoice_attempts
         WHERE tenant_id = $1
           AND status = 'succeeded'
           AND ar_invoice_id IS NULL",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    if let Some((attempt_id, cycle_key)) = orphaned.first() {
        return Err(InvariantViolation::SucceededWithoutInvoice {
            attempt_id: *attempt_id,
            cycle_key: cycle_key.clone(),
        });
    }

    Ok(())
}

/// Run all Subscriptions invariant checks
///
/// **Convenience function** that runs all Subscription-specific invariant assertions.
///
/// **Returns:** Ok(()) if all invariants hold, Err on first violation
pub async fn assert_all_invariants(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(), InvariantViolation> {
    assert_no_duplicate_cycle_attempts(pool, tenant_id).await?;
    assert_one_invoice_per_cycle(pool, tenant_id).await?;
    assert_no_attempts_after_terminal(pool, tenant_id).await?;
    assert_succeeded_has_invoice_id(pool, tenant_id).await?;
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
        let subscription_id = Uuid::nil();

        let violation = InvariantViolation::DuplicateCycleAttempts {
            tenant_id: "tenant-123".to_string(),
            subscription_id,
            cycle_key: "2026-02".to_string(),
            count: 2,
        };
        assert!(violation.to_string().contains("Duplicate cycle attempts"));
        assert!(violation.to_string().contains("tenant-123"));
        assert!(violation.to_string().contains("2026-02"));

        let violation = InvariantViolation::MultipleCycleInvoices {
            tenant_id: "tenant-123".to_string(),
            subscription_id,
            cycle_key: "2026-02".to_string(),
            invoice_count: 3,
        };
        assert!(violation
            .to_string()
            .contains("Multiple invoices for cycle"));
        assert!(violation.to_string().contains("invoice_count=3"));

        let violation = InvariantViolation::SucceededWithoutInvoice {
            attempt_id: Uuid::nil(),
            cycle_key: "2026-02".to_string(),
        };
        assert!(violation
            .to_string()
            .contains("marked 'succeeded' but has no ar_invoice_id"));
    }

    #[test]
    fn test_invariant_violation_from_sqlx_error() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let violation: InvariantViolation = sqlx_err.into();
        assert!(matches!(violation, InvariantViolation::DatabaseError(_)));
    }
}
