//! Payments Lifecycle Module (Phase 15 - bd-3lm)
//!
//! **Mutation Ownership:** This module owns ALL payment attempt status mutations.
//! Handlers MUST call lifecycle functions. Direct SQL updates are forbidden.
//!
//! **Critical Invariant (ChatGPT):**
//! Guards validate transitions ONLY. ZERO side effects in guards.
//!
//! **Execution Pattern:**
//! 1. Guard validates transition (pure logic, no I/O except DB read)
//! 2. Lifecycle function mutates state (after guard approval)
//! 3. Lifecycle function emits events (after mutation succeeds)
//!
//! **Payment Attempt State Machine:**
//! ```text
//! ATTEMPTING ──> SUCCEEDED
//!   |
//!   ├──> FAILED_RETRY ──> ATTEMPTING (retry window)
//!   |
//!   ├──> FAILED_FINAL (terminal)
//!   |
//!   └──> UNKNOWN ──> reconciliation ──> SUCCEEDED / FAILED_*
//! ```
//!
//! **UNKNOWN Protocol (Scaffolded):**
//! - UNKNOWN blocks retries (bd-1it)
//! - UNKNOWN blocks subscription suspension (bd-184)
//! - Full reconciliation workflow in bd-2uw

use sqlx::{PgPool, Postgres, Transaction};
use std::fmt;
use tracing::{info, warn};
use uuid::Uuid;

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
    /// Payment attempt not found
    AttemptNotFound(Uuid),
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
            Self::AttemptNotFound(id) => write!(f, "Payment attempt not found: {}", id),
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
// Payment Attempt Status Constants (matching payment_attempt_status enum)
// ============================================================================

pub mod status {
    pub const ATTEMPTING: &str = "attempting";
    pub const SUCCEEDED: &str = "succeeded";
    pub const FAILED_RETRY: &str = "failed_retry";
    pub const FAILED_FINAL: &str = "failed_final";
    pub const UNKNOWN: &str = "unknown"; // Phase 15 UNKNOWN protocol
}

// ============================================================================
// Transition Guards (Phase 15 Critical Invariant: ZERO side effects)
// ============================================================================

/// Validate transition from current status to target status
///
/// **Critical Invariant:** This function performs VALIDATION ONLY.
/// - NO event emission
/// - NO HTTP calls (including PSP calls)
/// - NO ledger posts
/// - NO webhook signature verification
/// - NO external I/O
/// - Returns Result<(), TransitionError> ONLY
///
/// Side effects happen in the calling lifecycle function AFTER guard approval.
async fn validate_transition(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
    to_status: &str,
) -> Result<(), TransitionError> {
    // Fetch current payment attempt status (cast enum to TEXT)
    let current_status: Option<String> = sqlx::query_scalar(
        "SELECT status::text FROM payment_attempts WHERE id = $1"
    )
    .bind(attempt_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| TransitionError::DatabaseError(e.to_string()))?;

    let from_status = current_status
        .ok_or(TransitionError::AttemptNotFound(attempt_id))?;

    // State machine rules
    let is_valid = match (from_status.as_str(), to_status) {
        // ATTEMPTING can transition to all terminal/intermediate states
        (status::ATTEMPTING, status::SUCCEEDED) => true,
        (status::ATTEMPTING, status::FAILED_RETRY) => true,
        (status::ATTEMPTING, status::FAILED_FINAL) => true,
        (status::ATTEMPTING, status::UNKNOWN) => true,

        // FAILED_RETRY can transition back to ATTEMPTING (retry window opens)
        (status::FAILED_RETRY, status::ATTEMPTING) => true,

        // UNKNOWN can transition to terminal states (after reconciliation)
        (status::UNKNOWN, status::SUCCEEDED) => true,
        (status::UNKNOWN, status::FAILED_RETRY) => true,
        (status::UNKNOWN, status::FAILED_FINAL) => true,

        // Terminal states (SUCCEEDED, FAILED_FINAL) have no outgoing transitions
        (status::SUCCEEDED, _) => false,
        (status::FAILED_FINAL, _) => false,

        // All other transitions are illegal
        _ => false,
    };

    if !is_valid {
        // Canonical log schema: reject decision (9 fields)
        warn!(
            module = "payments",
            entity_type = "payment_attempt",
            entity_id = %attempt_id,
            from_state = %from_status,
            to_state = to_status,
            decision = "reject",
            reason_code = "illegal_transition",
            message = "Payment attempt transition rejected by state machine",
            context = ?serde_json::json!({
                "from": from_status.clone(),
                "to": to_status,
                "state_machine": "payment_attempt"
            }),
            "Payment attempt transition rejected"
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

    // Canonical log schema: accept decision (9 fields)
    info!(
        module = "payments",
        entity_type = "payment_attempt",
        entity_id = %attempt_id,
        from_state = %from_status,
        to_state = to_status,
        decision = "accept",
        reason_code = "valid_transition",
        message = "Payment attempt transition accepted by state machine",
        context = ?serde_json::json!({
            "from": from_status,
            "to": to_status,
            "state_machine": "payment_attempt"
        }),
        "Payment attempt transition accepted"
    );

    Ok(())
}

// ============================================================================
// Lifecycle Functions (Pattern: guard → mutate → emit)
// ============================================================================

/// Transition payment attempt to SUCCEEDED status
///
/// **Pattern:** guard → mutate → emit
/// - Guard validates transition (zero side effects)
/// - Mutate updates payment_attempts.status
/// - Emit events (NOT YET IMPLEMENTED - placeholder for bd-1wg or bd-1it)
///
/// **Usage:**
/// ```ignore
/// let pool = /* ... */;
/// let attempt_id = Uuid::new_v4();
/// transition_to_succeeded(&pool, attempt_id, "PSP confirmed payment").await?;
/// ```
pub async fn transition_to_succeeded(
    pool: &PgPool,
    attempt_id: Uuid,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition (ZERO side effects)
    validate_transition(&mut tx, attempt_id, status::SUCCEEDED).await?;

    // 2. MUTATE: Update payment attempt status (after guard approval)
    sqlx::query("UPDATE payment_attempts SET status = $1::payment_attempt_status, completed_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(status::SUCCEEDED)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here (after mutation succeeds)
    // 3. EMIT: Event emission (future bead - bd-1wg)
    // When implemented, use enqueue_event_tx() for atomicity:
    // use crate::events::outbox::enqueue_event_tx;
    // let envelope = create_payments_envelope(...payment_succeeded_payload);
    // enqueue_event_tx(&mut tx, "payment.succeeded", &envelope).await?;
    // CRITICAL: Emit BEFORE tx.commit() to ensure atomicity

    tx.commit().await?;
    Ok(())
}

/// Transition payment attempt to FAILED_RETRY status
///
/// **Pattern:** guard → mutate → emit
/// **Note:** FAILED_RETRY allows retry via transition back to ATTEMPTING (bd-1it)
pub async fn transition_to_failed_retry(
    pool: &PgPool,
    attempt_id: Uuid,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, attempt_id, status::FAILED_RETRY).await?;

    // 2. MUTATE: Update payment attempt status
    sqlx::query("UPDATE payment_attempts SET status = $1::payment_attempt_status, completed_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(status::FAILED_RETRY)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // 3. EMIT: Event emission (future bead - bd-1wg)
    // When implemented, use enqueue_event_tx(&mut tx, ...) BEFORE tx.commit()

    tx.commit().await?;
    Ok(())
}

/// Transition payment attempt to FAILED_FINAL status
///
/// **Pattern:** guard → mutate → emit
/// **Note:** FAILED_FINAL is terminal (no retry possible)
pub async fn transition_to_failed_final(
    pool: &PgPool,
    attempt_id: Uuid,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, attempt_id, status::FAILED_FINAL).await?;

    // 2. MUTATE: Update payment attempt status
    sqlx::query("UPDATE payment_attempts SET status = $1::payment_attempt_status, completed_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(status::FAILED_FINAL)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // 3. EMIT: Event emission (future bead - bd-1wg)
    // When implemented, use enqueue_event_tx(&mut tx, ...) BEFORE tx.commit()

    tx.commit().await?;
    Ok(())
}

/// Transition payment attempt to UNKNOWN status
///
/// **Pattern:** guard → mutate → emit
///
/// **UNKNOWN Protocol (Scaffolded):**
/// - Webhook provided ambiguous result (network timeout, PSP error, etc.)
/// - Blocks retry scheduling (bd-1it) - cannot retry while UNKNOWN
/// - Blocks subscription suspension (bd-184) - customer not at fault
/// - Requires reconciliation workflow (bd-2uw) to resolve to terminal state
///
/// **Reconciliation (bd-2uw):**
/// - Poll PSP for actual payment status
/// - Transition UNKNOWN → SUCCEEDED or FAILED_* based on PSP response
/// - Deterministic (always resolves to same terminal state)
pub async fn transition_to_unknown(
    pool: &PgPool,
    attempt_id: Uuid,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, attempt_id, status::UNKNOWN).await?;

    // 2. MUTATE: Update payment attempt status
    sqlx::query("UPDATE payment_attempts SET status = $1::payment_attempt_status, completed_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(status::UNKNOWN)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // 3. EMIT: Event emission + reconciliation trigger (bd-2uw)
    // When implemented, use enqueue_event_tx(&mut tx, ...) BEFORE tx.commit()

    tx.commit().await?;
    Ok(())
}

/// Transition payment attempt back to ATTEMPTING status (retry window)
///
/// **Pattern:** guard → mutate → emit
///
/// **Retry Windows (bd-1it):**
/// - From FAILED_RETRY only (not from UNKNOWN or terminal states)
/// - Triggered by retry scheduler at +3d or +7d windows
/// - New attempt row must be created (different attempt_no)
pub async fn transition_to_attempting(
    pool: &PgPool,
    attempt_id: Uuid,
    _reason: &str,
) -> Result<(), LifecycleError> {
    let mut tx = pool.begin().await?;

    // 1. GUARD: Validate transition
    validate_transition(&mut tx, attempt_id, status::ATTEMPTING).await?;

    // 2. MUTATE: Update payment attempt status
    sqlx::query("UPDATE payment_attempts SET status = $1::payment_attempt_status, attempted_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(status::ATTEMPTING)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

    // 3. EMIT: Side effects go here
    // 3. EMIT: Event emission (future bead - bd-1it)
    // When implemented, use enqueue_event_tx(&mut tx, ...) BEFORE tx.commit()

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_error_display() {
        let err = TransitionError::IllegalTransition {
            from: "attempting".to_string(),
            to: "failed_final".to_string(),
            reason: "Must transition through failed_retry first".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Illegal transition from 'attempting' to 'failed_final': Must transition through failed_retry first"
        );
    }

    #[test]
    fn test_attempt_not_found_error() {
        let id = Uuid::nil();
        let err = TransitionError::AttemptNotFound(id);
        assert_eq!(err.to_string(), format!("Payment attempt not found: {}", id));
    }

    #[test]
    fn test_status_constants() {
        assert_eq!(status::ATTEMPTING, "attempting");
        assert_eq!(status::SUCCEEDED, "succeeded");
        assert_eq!(status::FAILED_RETRY, "failed_retry");
        assert_eq!(status::FAILED_FINAL, "failed_final");
        assert_eq!(status::UNKNOWN, "unknown");
    }
}
