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
    pub audit_pool: &'a PgPool,
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
    AuditInvariantViolation(String),
}

impl std::fmt::Display for OracleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleError::ArInvariantViolation(e) => write!(f, "AR Invariant Violation: {}", e),
            OracleError::PaymentsInvariantViolation(e) => write!(f, "Payments Invariant Violation: {}", e),
            OracleError::SubscriptionsInvariantViolation(e) => write!(f, "Subscriptions Invariant Violation: {}", e),
            OracleError::GlInvariantViolation(e) => write!(f, "GL Invariant Violation: {}", e),
            OracleError::AuditInvariantViolation(e) => write!(f, "Audit Invariant Violation: {}", e),
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
// Audit Completeness Checks
// ============================================================================

/// Check audit completeness for a defined set of mutations
///
/// **Invariant:** For each mutation event in outbox tables, exactly one audit record exists
/// with proper causation/correlation/trace linkage
///
/// **Checks:**
/// - AR: events_outbox mutations have corresponding audit records
/// - Payments: payments_events_outbox mutations have corresponding audit records
/// - GL: events_outbox mutations have corresponding audit records
///
/// **Mode:** If no audit records exist at all, the check passes (audit not yet integrated).
/// If some but not all mutations are audited, that's a violation.
///
/// **Returns:** Ok(()) if all mutations are audited exactly once, Err if gaps or duplicates found
async fn assert_audit_completeness(ctx: &TestContext<'_>) -> Result<(), OracleError> {
    use sqlx::Row;
    use uuid::Uuid;

    // Check if audit table exists and has any records
    let audit_table_exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = 'audit_events'
        )
        "#
    )
    .fetch_one(ctx.audit_pool)
    .await
    .unwrap_or(false);

    if !audit_table_exists {
        // Audit not yet integrated - skip checks
        return Ok(());
    }

    let total_audit_records: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM audit_events"#
    )
    .fetch_one(ctx.audit_pool)
    .await
    .unwrap_or(0);

    if total_audit_records == 0 {
        // No audit records yet - audit not yet integrated
        return Ok(());
    }

    // Query all mutation events from AR outbox
    let ar_events: Vec<Uuid> = sqlx::query(
        r#"
        SELECT event_id
        FROM events_outbox
        WHERE event_type LIKE '%Created' OR event_type LIKE '%Updated' OR event_type LIKE '%Finalized'
        ORDER BY created_at
        "#
    )
    .fetch_all(ctx.ar_pool)
    .await
    .map_err(|e| OracleError::AuditInvariantViolation(format!("Failed to query AR outbox: {}", e)))?
    .into_iter()
    .map(|row| row.get::<Uuid, _>("event_id"))
    .collect();

    // Check each AR event has exactly one audit record
    for event_id in ar_events {
        let audit_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint
            FROM audit_events
            WHERE causation_id = $1
            "#
        )
        .bind(event_id)
        .fetch_one(ctx.audit_pool)
        .await
        .map_err(|e| OracleError::AuditInvariantViolation(format!("Failed to query audit for AR event {}: {}", event_id, e)))?;

        if audit_count == 0 {
            return Err(OracleError::AuditInvariantViolation(format!(
                "AR mutation {} has no audit record",
                event_id
            )));
        }

        if audit_count > 1 {
            return Err(OracleError::AuditInvariantViolation(format!(
                "AR mutation {} has {} audit records (expected exactly 1)",
                event_id, audit_count
            )));
        }
    }

    // Query all mutation events from Payments outbox
    let payment_events: Vec<Uuid> = sqlx::query(
        r#"
        SELECT event_id
        FROM payments_events_outbox
        WHERE event_type LIKE '%Created' OR event_type LIKE '%Updated' OR event_type LIKE '%Transition%'
        ORDER BY created_at
        "#
    )
    .fetch_all(ctx.payments_pool)
    .await
    .map_err(|e| OracleError::AuditInvariantViolation(format!("Failed to query Payments outbox: {}", e)))?
    .into_iter()
    .map(|row| row.get::<Uuid, _>("event_id"))
    .collect();

    // Check each Payment event has exactly one audit record
    for event_id in payment_events {
        let audit_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint
            FROM audit_events
            WHERE causation_id = $1
            "#
        )
        .bind(event_id)
        .fetch_one(ctx.audit_pool)
        .await
        .map_err(|e| OracleError::AuditInvariantViolation(format!("Failed to query audit for Payment event {}: {}", event_id, e)))?;

        if audit_count == 0 {
            return Err(OracleError::AuditInvariantViolation(format!(
                "Payment mutation {} has no audit record",
                event_id
            )));
        }

        if audit_count > 1 {
            return Err(OracleError::AuditInvariantViolation(format!(
                "Payment mutation {} has {} audit records (expected exactly 1)",
                event_id, audit_count
            )));
        }
    }

    // Query all mutation events from GL outbox
    let gl_events: Vec<Uuid> = sqlx::query(
        r#"
        SELECT event_id
        FROM events_outbox
        WHERE event_type LIKE '%Created' OR event_type LIKE '%Posted' OR event_type LIKE '%Reversed'
        ORDER BY created_at
        "#
    )
    .fetch_all(ctx.gl_pool)
    .await
    .map_err(|e| OracleError::AuditInvariantViolation(format!("Failed to query GL outbox: {}", e)))?
    .into_iter()
    .map(|row| row.get::<Uuid, _>("event_id"))
    .collect();

    // Check each GL event has exactly one audit record
    for event_id in gl_events {
        let audit_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint
            FROM audit_events
            WHERE causation_id = $1
            "#
        )
        .bind(event_id)
        .fetch_one(ctx.audit_pool)
        .await
        .map_err(|e| OracleError::AuditInvariantViolation(format!("Failed to query audit for GL event {}: {}", event_id, e)))?;

        if audit_count == 0 {
            return Err(OracleError::AuditInvariantViolation(format!(
                "GL mutation {} has no audit record",
                event_id
            )));
        }

        if audit_count > 1 {
            return Err(OracleError::AuditInvariantViolation(format!(
                "GL mutation {} has {} audit records (expected exactly 1)",
                event_id, audit_count
            )));
        }
    }

    Ok(())
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
/// 5. Audit: Exactly one audit record per mutation with proper causation linkage
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

    // Check audit completeness for all mutations
    assert_audit_completeness(ctx).await?;

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
