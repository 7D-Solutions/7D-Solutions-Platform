//! Cross-Module Oracle - Invariant Enforcement Spine (Phase 15 - bd-3rc.9)
//!
//! **Purpose:** Centralized entrypoint for all module-level invariant checks
//!
//! **Pattern:** Single function that calls all 4 module invariants (AR, Payments, Subscriptions, GL)
//!
//! **Usage:**
//! ```rust
//! let ctx = TestContext { ar_pool, payments_pool, subscriptions_pool, gl_pool, app_id, tenant_id };
//! assert_cross_module_invariants(&ctx).await?;
//! ```
//!
//! **ChatGPT Requirement:** Oracle spine extraction for Phase 15 oracle certification

use sqlx::PgPool;

// ============================================================================
// Test Context
// ============================================================================

/// Test context containing all database pools and tenant identifiers
pub struct TestContext<'a> {
    pub ar_pool: &'a PgPool,
    pub payments_pool: &'a PgPool,
    pub subscriptions_pool: &'a PgPool,
    pub gl_pool: &'a PgPool,
    pub app_id: &'a str,
    pub tenant_id: &'a str,
}

// ============================================================================
// Oracle Error Type
// ============================================================================

/// Unified error type for cross-module invariant violations
#[derive(Debug)]
pub enum OracleError {
    ArInvariantViolation(ar_rs::invariants::InvariantViolation),
    PaymentsInvariantViolation(payments_rs::invariants::InvariantViolation),
    SubscriptionsInvariantViolation(subscriptions_rs::invariants::InvariantViolation),
    GlInvariantViolation(gl_rs::invariants::InvariantViolation),
}

impl std::fmt::Display for OracleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleError::ArInvariantViolation(e) => write!(f, "AR Invariant Violation: {}", e),
            OracleError::PaymentsInvariantViolation(e) => write!(f, "Payments Invariant Violation: {}", e),
            OracleError::SubscriptionsInvariantViolation(e) => write!(f, "Subscriptions Invariant Violation: {}", e),
            OracleError::GlInvariantViolation(e) => write!(f, "GL Invariant Violation: {}", e),
        }
    }
}

impl std::error::Error for OracleError {}

impl From<ar_rs::invariants::InvariantViolation> for OracleError {
    fn from(e: ar_rs::invariants::InvariantViolation) -> Self {
        OracleError::ArInvariantViolation(e)
    }
}

impl From<payments_rs::invariants::InvariantViolation> for OracleError {
    fn from(e: payments_rs::invariants::InvariantViolation) -> Self {
        OracleError::PaymentsInvariantViolation(e)
    }
}

impl From<subscriptions_rs::invariants::InvariantViolation> for OracleError {
    fn from(e: subscriptions_rs::invariants::InvariantViolation) -> Self {
        OracleError::SubscriptionsInvariantViolation(e)
    }
}

impl From<gl_rs::invariants::InvariantViolation> for OracleError {
    fn from(e: gl_rs::invariants::InvariantViolation) -> Self {
        OracleError::GlInvariantViolation(e)
    }
}

// ============================================================================
// Oracle Entrypoint
// ============================================================================

/// Assert all cross-module invariants
///
/// **Invariants Checked:**
/// 1. AR: No duplicate attempts, attempt count limits, no attempts after terminal
/// 2. Payments: No duplicate attempts, attempt count limits, UNKNOWN protocol compliance
/// 3. Subscriptions: No duplicate cycle attempts, one invoice per cycle, no attempts after terminal
/// 4. GL: All entries balanced, no duplicate postings, valid account references
///
/// **Usage:** Call after every operation in E2E tests
///
/// **Returns:** Ok(()) if all invariants pass, Err(OracleError) if any violation
pub async fn assert_cross_module_invariants(ctx: &TestContext<'_>) -> Result<(), OracleError> {
    // Call all 4 module invariants from bd-35x
    ar_rs::invariants::assert_all_invariants(ctx.ar_pool, ctx.app_id).await?;
    payments_rs::invariants::assert_all_invariants(ctx.payments_pool, ctx.app_id).await?;
    subscriptions_rs::invariants::assert_all_invariants(ctx.subscriptions_pool, ctx.tenant_id).await?;
    gl_rs::invariants::assert_all_invariants(ctx.gl_pool, ctx.tenant_id).await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_error_display() {
        // Test that OracleError variants display correctly
        let ar_error = OracleError::ArInvariantViolation(
            ar_rs::invariants::InvariantViolation::DuplicateAttempts {
                app_id: "test-app".to_string(),
                invoice_id: 123,
                attempt_no: 0,
                count: 2,
            }
        );

        let display = format!("{}", ar_error);
        assert!(display.contains("AR Invariant Violation"));
    }
}
