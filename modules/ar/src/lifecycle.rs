//! AR Lifecycle Module (Phase 15 - bd-1w7)
//!
//! **Mutation Ownership:** This module owns ALL invoice status mutations.
//! Routes/handlers MUST call lifecycle functions. Direct SQL updates are forbidden.
//!
//! **Critical Invariant (ChatGPT):**
//! Guards validate transitions ONLY. ZERO side effects in guards.
//!
//! **Execution Pattern:**
//! 1. Guard validates transition (pure logic, no I/O)
//! 2. Lifecycle function mutates state (after guard approval)
//! 3. Lifecycle function emits events (after mutation succeeds)
//!
//! **Invoice State Machine:**
//! ```text
//! OPEN ──> ATTEMPTING ──> PAID
//!   |                       |
//!   └──> FAILED_FINAL <─────┘
//! ```

use sqlx::{PgPool, Postgres, Transaction};
use std::fmt;
use tracing::{info, warn};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// Transition is not allowed from current state
    IllegalTransition {
        from: String,
        to: String,
        reason: String,
    },
    /// Invoice not found
    InvoiceNotFound(i32),
    /// Database error during validation
    DatabaseError(String),
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IllegalTransition { from, to, reason } => {
                write!(
                    f,
                    "Illegal transition from '{}' to '{}': {}",
                    from, to, reason
                )
            }
            Self::InvoiceNotFound(id) => write!(f, "Invoice not found: {}", id),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for TransitionError {}

#[derive(Debug)]
pub enum LifecycleError {
    TransitionError(TransitionError),
    DatabaseError(sqlx::Error),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransitionError(e) => write!(f, "Transition error: {}", e),
            Self::DatabaseError(e) => write!(f, "Database error: {}", e),
        }
    }
}

impl std::error::Error for LifecycleError {}

impl From<TransitionError> for LifecycleError {
    fn from(e: TransitionError) -> Self {
        Self::TransitionError(e)
    }
}

impl From<sqlx::Error> for LifecycleError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e)
    }
}

// ============================================================================
// Invoice Status Constants (matching existing DB schema)
// ============================================================================

pub mod status {
    pub const DRAFT: &str = "draft";
    pub const OPEN: &str = "open";
    pub const PAID: &str = "paid";
    pub const VOID: &str = "void";
    pub const UNCOLLECTIBLE: &str = "uncollectible";
    // Phase 15 lifecycle states
    pub const ATTEMPTING: &str = "attempting";
    pub const FAILED_FINAL: &str = "failed_final";
}

// ============================================================================
// Transition Guards (Phase 15 Critical Invariant: ZERO side effects)
// ============================================================================

/// Validate transition from current status to target status
///
/// **Critical Invariant:** This function performs VALIDATION ONLY.
/// - NO event emission
/// - NO HTTP calls
/// - NO ledger posts
/// - NO external I/O
/// - Returns Result<(), TransitionError> ONLY
///
/// Side effects happen in the calling lifecycle function AFTER guard approval.
async fn validate_transition(
    tx: &mut Transaction<'_, Postgres>,
    invoice_id: i32,
    app_id: &str,
    to_status: &str,
) -> Result<(), TransitionError> {
    // Fetch current invoice status (tenant-scoped)
    let current_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1 AND app_id = $2")
            .bind(invoice_id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| TransitionError::DatabaseError(e.to_string()))?;

    let from_status = current_status.ok_or(TransitionError::InvoiceNotFound(invoice_id))?;

    // State machine rules
    let is_valid = match (from_status.as_str(), to_status) {
        // OPEN can transition to ATTEMPTING or VOID
        (status::OPEN, status::ATTEMPTING) => true,
        (status::OPEN, status::VOID) => true,

        // ATTEMPTING can transition to PAID or FAILED_FINAL
        (status::ATTEMPTING, status::PAID) => true,
        (status::ATTEMPTING, status::FAILED_FINAL) => true,

        // PAID and FAILED_FINAL are terminal (no outgoing transitions)
        (status::PAID, _) => false,
        (status::FAILED_FINAL, _) => false,

        // All other transitions are illegal
        _ => false,
    };

    if !is_valid {
        // Canonical log schema: reject decision
        warn!(
            module = "ar",
            entity_type = "invoice",
            entity_id = invoice_id,
            from_state = %from_status,
            to_state = to_status,
            decision = "reject",
            reason_code = "illegal_transition",
            "Invoice transition rejected by state machine"
        );

        return Err(TransitionError::IllegalTransition {
            from: from_status.clone(),
            to: to_status.to_string(),
            reason: format!(
                "State machine does not allow transition from {} to {}",
                from_status, to_status
            ),
        });
    }

    // Canonical log schema: accept decision
    info!(
        module = "ar",
        entity_type = "invoice",
        entity_id = invoice_id,
        from_state = %from_status,
        to_state = to_status,
        decision = "accept",
        reason_code = "valid_transition",
        "Invoice transition accepted by state machine"
    );

    Ok(())
}

// ============================================================================
// Lifecycle Functions (Pattern: guard → mutate → emit)
// ============================================================================

/// Transition invoice to ATTEMPTING status
///
/// **Pattern:** guard → mutate → emit
/// - Guard validates transition (zero side effects)
/// - Mutate updates invoice status
/// - Emit events (NOT YET IMPLEMENTED - placeholder for future beads)
///
/// **Usage:**
/// ```rust,ignore
/// let pool = todo!();
/// transition_to_attempting(&pool, invoice_id, "app-demo", "Initiating payment collection").await?;
/// ```
pub async fn transition_to_attempting(
    pool: &PgPool,
    invoice_id: i32,
    app_id: &str,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition (ZERO side effects)
    validate_transition(&mut tx, invoice_id, app_id, status::ATTEMPTING).await?;

    // 2. MUTATE: Update invoice status (after guard approval, tenant-scoped)
    sqlx::query("UPDATE ar_invoices SET status = $1 WHERE id = $2 AND app_id = $3")
        .bind(status::ATTEMPTING)
        .bind(invoice_id)
        .bind(app_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here (after mutation succeeds)
    // TODO: emit_invoice_attempting_event (future bead - bd-3fo or bd-8ev)

    tx.commit().await?;
    Ok(())
}

/// Transition invoice to PAID status
///
/// **Pattern:** guard → mutate → emit
pub async fn transition_to_paid(
    pool: &PgPool,
    invoice_id: i32,
    app_id: &str,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, invoice_id, app_id, status::PAID).await?;

    // 2. MUTATE: Update invoice status (tenant-scoped)
    sqlx::query("UPDATE ar_invoices SET status = $1, paid_at = CURRENT_TIMESTAMP WHERE id = $2 AND app_id = $3")
        .bind(status::PAID)
        .bind(invoice_id)
        .bind(app_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // TODO: emit_invoice_paid_event (future bead)

    tx.commit().await?;
    Ok(())
}

/// Transition invoice to FAILED_FINAL status
///
/// **Pattern:** guard → mutate → emit
pub async fn transition_to_failed_final(
    pool: &PgPool,
    invoice_id: i32,
    app_id: &str,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, invoice_id, app_id, status::FAILED_FINAL).await?;

    // 2. MUTATE: Update invoice status (tenant-scoped)
    sqlx::query("UPDATE ar_invoices SET status = $1 WHERE id = $2 AND app_id = $3")
        .bind(status::FAILED_FINAL)
        .bind(invoice_id)
        .bind(app_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // TODO: emit_invoice_failed_final_event (future bead)

    tx.commit().await?;
    Ok(())
}

/// Transition invoice to VOID status
///
/// **Pattern:** guard → mutate → emit
pub async fn transition_to_void(
    pool: &PgPool,
    invoice_id: i32,
    app_id: &str,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, invoice_id, app_id, status::VOID).await?;

    // 2. MUTATE: Update invoice status (tenant-scoped)
    sqlx::query("UPDATE ar_invoices SET status = $1 WHERE id = $2 AND app_id = $3")
        .bind(status::VOID)
        .bind(invoice_id)
        .bind(app_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // TODO: emit_invoice_void_event (future bead)

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_error_display() {
        let err = TransitionError::IllegalTransition {
            from: "open".to_string(),
            to: "paid".to_string(),
            reason: "Cannot skip ATTEMPTING".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Illegal transition from 'open' to 'paid': Cannot skip ATTEMPTING"
        );
    }

    #[test]
    fn test_invoice_not_found_error() {
        let err = TransitionError::InvoiceNotFound(123);
        assert_eq!(err.to_string(), "Invoice not found: 123");
    }
}
