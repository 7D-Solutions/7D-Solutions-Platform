//! UNKNOWN Protocol Reconciliation (Phase 15 - bd-2uw)
//!
//! **Purpose:** Resolve payment attempts stuck in UNKNOWN state by polling PSP.
//!
//! **UNKNOWN Protocol Semantics:**
//! - UNKNOWN occurs when webhook result is ambiguous (network timeout, PSP error, etc.)
//! - UNKNOWN blocks retry scheduling (bd-1it) - cannot retry while status is ambiguous
//! - UNKNOWN blocks subscription suspension (bd-184) - customer not at fault for PSP issues
//! - UNKNOWN must be resolved via reconciliation before downstream lifecycle actions
//!
//! **Reconciliation Workflow:**
//! 1. Lock attempt row with SELECT FOR UPDATE
//! 2. Check if status is still UNKNOWN (idempotency guard)
//! 3. Poll PSP for actual payment status (bounded retry: max 3 attempts)
//! 4. Resolve UNKNOWN → terminal state via lifecycle functions
//! 5. Emit events (after successful resolution)
//!
//! **Critical Invariants (ChatGPT):**
//! - Deterministic: Same processor_payment_id always resolves to same terminal state
//! - Idempotent: Safe to call reconcile multiple times
//! - Bounded: Max 3 PSP polling attempts with exponential backoff
//! - Lifecycle integration: Use lifecycle::transition_to_* functions (NO direct SQL)

use crate::lifecycle::{status, LifecycleError};
use crate::processor::MockPaymentProcessor;
use sqlx::PgPool;
use std::fmt;
use tracing::{info, warn};
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug)]
pub enum ReconciliationError {
    /// Payment attempt not found in database
    AttemptNotFound(Uuid),
    /// Attempt is not in UNKNOWN state (cannot reconcile)
    NotInUnknownState {
        attempt_id: Uuid,
        current_status: String,
    },
    /// PSP polling failed after multiple attempts
    PspPollingFailed {
        attempt_count: i32,
        last_error: String,
    },
    /// Max PSP polling retries exceeded
    MaxRetriesExceeded { attempt_id: Uuid },
    /// Missing processor_payment_id (cannot query PSP)
    MissingProcessorPaymentId(Uuid),
    /// Database error
    DatabaseError(String),
    /// Lifecycle transition error
    LifecycleError(LifecycleError),
}

impl fmt::Display for ReconciliationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AttemptNotFound(id) => write!(f, "Payment attempt not found: {}", id),
            Self::NotInUnknownState {
                attempt_id,
                current_status,
            } => write!(
                f,
                "Attempt {} is not in UNKNOWN state (current: {})",
                attempt_id, current_status
            ),
            Self::PspPollingFailed {
                attempt_count,
                last_error,
            } => write!(
                f,
                "PSP polling failed after {} attempts: {}",
                attempt_count, last_error
            ),
            Self::MaxRetriesExceeded { attempt_id } => write!(
                f,
                "Max PSP polling retries exceeded for attempt {}",
                attempt_id
            ),
            Self::MissingProcessorPaymentId(id) => {
                write!(f, "Missing processor_payment_id for attempt {}", id)
            }
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            Self::LifecycleError(e) => write!(f, "Lifecycle error: {}", e),
        }
    }
}

impl std::error::Error for ReconciliationError {}

impl From<sqlx::Error> for ReconciliationError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

impl From<LifecycleError> for ReconciliationError {
    fn from(e: LifecycleError) -> Self {
        Self::LifecycleError(e)
    }
}

// ============================================================================
// Result Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationResult {
    /// Successfully resolved UNKNOWN to terminal state
    Resolved { from: String, to: String },
    /// Attempt already resolved (not in UNKNOWN state)
    AlreadyResolved { current_status: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PspPaymentStatus {
    /// Payment succeeded at PSP
    Succeeded,
    /// Payment failed at PSP (transient failure - can retry)
    FailedRetry { code: String, message: String },
    /// Payment failed at PSP (permanent failure - cannot retry)
    FailedFinal { code: String, message: String },
    /// PSP still doesn't know the status (rare)
    StillUnknown,
}

// ============================================================================
// Reconciliation Entry Point
// ============================================================================

/// Reconcile payment attempt in UNKNOWN state by polling PSP
///
/// **Pattern:** Lock → Check → Poll → Resolve → Emit → Commit
///
/// **Idempotency:** Returns `AlreadyResolved` if status != UNKNOWN (safe to call multiple times)
///
/// **Bounded Retry:** Max 3 PSP polling attempts with exponential backoff (1s, 2s, 4s)
///
/// **Usage:**
/// ```ignore
/// use payments::reconciliation::{reconcile_unknown_attempt, ReconciliationResult};
///
/// let result = reconcile_unknown_attempt(&pool, attempt_id).await?;
/// match result {
///     ReconciliationResult::Resolved { from, to } => {
///         println!("Resolved {} -> {}", from, to);
///     }
///     ReconciliationResult::AlreadyResolved { current_status } => {
///         println!("Already resolved to {}", current_status);
///     }
/// }
/// ```
pub async fn reconcile_unknown_attempt(
    pool: &PgPool,
    attempt_id: Uuid,
) -> Result<ReconciliationResult, ReconciliationError> {
    let mut tx = pool.begin().await?;

    // ========================================================================
    // STEP 1: Lock attempt row (prevents concurrent reconciliation)
    // ========================================================================
    let attempt: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT status::text, processor_payment_id FROM payment_attempts WHERE id = $1 FOR UPDATE",
    )
    .bind(attempt_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (current_status, processor_payment_id) =
        attempt.ok_or(ReconciliationError::AttemptNotFound(attempt_id))?;

    // ========================================================================
    // STEP 2: Idempotency check - Return early if not UNKNOWN
    // ========================================================================
    if current_status != status::UNKNOWN {
        info!(
            module = "payments",
            entity_type = "payment_attempt",
            entity_id = %attempt_id,
            from_state = status::UNKNOWN,
            to_state = %current_status,
            decision = "skip",
            reason_code = "already_resolved",
            message = "Reconciliation skipped (attempt already resolved)",
            context = ?serde_json::json!({
                "current_status": current_status,
            }),
            "Reconciliation skipped - attempt already resolved"
        );

        tx.commit().await?;
        return Ok(ReconciliationResult::AlreadyResolved { current_status });
    }

    // ========================================================================
    // STEP 3: Poll PSP for actual payment status
    // ========================================================================
    let processor_payment_id =
        processor_payment_id.ok_or(ReconciliationError::MissingProcessorPaymentId(attempt_id))?;

    let psp_status = poll_psp_status_with_retry(&processor_payment_id, 3).await?;

    // ========================================================================
    // STEP 4: Resolve UNKNOWN to terminal state via lifecycle functions
    // ========================================================================
    //
    // **CRITICAL:** Use lifecycle functions (NOT direct SQL UPDATE)
    // This ensures lifecycle guards are enforced and events are emitted
    tx.commit().await?; // Commit lock transaction before calling lifecycle

    let target_status = match &psp_status {
        PspPaymentStatus::Succeeded => status::SUCCEEDED,
        PspPaymentStatus::FailedRetry { .. } => status::FAILED_RETRY,
        PspPaymentStatus::FailedFinal { .. } => status::FAILED_FINAL,
        PspPaymentStatus::StillUnknown => {
            warn!(
                module = "payments",
                entity_type = "payment_attempt",
                entity_id = %attempt_id,
                from_state = status::UNKNOWN,
                to_state = status::UNKNOWN,
                decision = "defer",
                reason_code = "psp_still_unknown",
                message = "PSP still does not know payment status - reconciliation deferred",
                context = ?serde_json::json!({
                    "processor_payment_id": processor_payment_id,
                }),
                "PSP still unknown - reconciliation deferred"
            );

            // PSP still doesn't know - do not change state
            return Ok(ReconciliationResult::AlreadyResolved {
                current_status: status::UNKNOWN.to_string(),
            });
        }
    };

    resolve_unknown_to_terminal(pool, attempt_id, target_status, &psp_status).await?;

    info!(
        module = "payments",
        entity_type = "payment_attempt",
        entity_id = %attempt_id,
        from_state = status::UNKNOWN,
        to_state = target_status,
        decision = "accept",
        reason_code = "psp_resolved",
        message = "Payment attempt reconciled via PSP query",
        context = ?serde_json::json!({
            "processor_payment_id": processor_payment_id,
            "psp_status": format!("{:?}", psp_status),
        }),
        "Payment attempt reconciled"
    );

    Ok(ReconciliationResult::Resolved {
        from: status::UNKNOWN.to_string(),
        to: target_status.to_string(),
    })
}

// ============================================================================
// PSP Polling (Bounded Retry)
// ============================================================================

/// Poll PSP for payment status with bounded retry
///
/// **Bounded Retry:** Max `max_attempts` with exponential backoff (1s, 2s, 4s)
///
/// **Usage:**
/// ```ignore
/// let psp_status = poll_psp_status_with_retry("pay_stripe_12345", 3).await?;
/// ```
async fn poll_psp_status_with_retry(
    processor_payment_id: &str,
    max_attempts: i32,
) -> Result<PspPaymentStatus, ReconciliationError> {
    let processor = MockPaymentProcessor::new();
    let mut last_error = String::new();

    for attempt in 1..=max_attempts {
        match processor.query_payment_status(processor_payment_id).await {
            Ok(status) => return Ok(status),
            Err(e) => {
                last_error = e.to_string();
                warn!(
                    "PSP polling attempt {}/{} failed: {}",
                    attempt, max_attempts, last_error
                );

                if attempt < max_attempts {
                    let backoff_ms = 2u64.pow((attempt - 1) as u32) * 1000; // 1s, 2s, 4s
                    tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }

    Err(ReconciliationError::PspPollingFailed {
        attempt_count: max_attempts,
        last_error,
    })
}

// ============================================================================
// Lifecycle Integration
// ============================================================================

/// Resolve UNKNOWN to terminal state via lifecycle functions
///
/// **Pattern:** Call lifecycle::transition_to_* (NO direct SQL UPDATE)
///
/// **Usage:**
/// ```ignore
/// resolve_unknown_to_terminal(pool, attempt_id, "succeeded", &psp_status).await?;
/// ```
async fn resolve_unknown_to_terminal(
    pool: &PgPool,
    attempt_id: Uuid,
    target_status: &str,
    psp_status: &PspPaymentStatus,
) -> Result<(), ReconciliationError> {
    // Fetch completed_at timestamp to calculate UNKNOWN duration (Phase 16: bd-1pw7)
    let completed_at: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT completed_at FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_optional(pool)
            .await?
            .flatten();

    let reason = match psp_status {
        PspPaymentStatus::Succeeded => "PSP confirmed payment succeeded".to_string(),
        PspPaymentStatus::FailedRetry { code, message } => {
            format!("PSP returned transient failure: {} - {}", code, message)
        }
        PspPaymentStatus::FailedFinal { code, message } => {
            format!("PSP returned permanent failure: {} - {}", code, message)
        }
        PspPaymentStatus::StillUnknown => "PSP still does not know payment status".to_string(),
    };

    // Call lifecycle functions (enforces guards, emits events)
    match target_status {
        status::SUCCEEDED => {
            crate::lifecycle::transition_to_succeeded(pool, attempt_id, &reason).await?;
        }
        status::FAILED_RETRY => {
            crate::lifecycle::transition_to_failed_retry(pool, attempt_id, &reason).await?;
        }
        status::FAILED_FINAL => {
            crate::lifecycle::transition_to_failed_final(pool, attempt_id, &reason).await?;
        }
        _ => {
            return Err(ReconciliationError::DatabaseError(format!(
                "Invalid target status for reconciliation: {}",
                target_status
            )));
        }
    }

    // Record UNKNOWN duration metric (Phase 16: bd-1pw7)
    if let Some(unknown_started_at) = completed_at {
        let now = chrono::Utc::now().naive_utc();
        let duration = now.signed_duration_since(unknown_started_at);
        let duration_seconds = duration.num_seconds() as f64;
        crate::metrics::record_unknown_duration(duration_seconds);
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconciliation_error_display() {
        let err = ReconciliationError::AttemptNotFound(Uuid::nil());
        assert_eq!(
            err.to_string(),
            format!("Payment attempt not found: {}", Uuid::nil())
        );

        let err = ReconciliationError::NotInUnknownState {
            attempt_id: Uuid::nil(),
            current_status: "succeeded".to_string(),
        };
        assert_eq!(
            err.to_string(),
            format!(
                "Attempt {} is not in UNKNOWN state (current: succeeded)",
                Uuid::nil()
            )
        );

        let err = ReconciliationError::PspPollingFailed {
            attempt_count: 3,
            last_error: "Network timeout".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "PSP polling failed after 3 attempts: Network timeout"
        );
    }

    #[test]
    fn test_reconciliation_result_equality() {
        let result1 = ReconciliationResult::Resolved {
            from: "unknown".to_string(),
            to: "succeeded".to_string(),
        };
        let result2 = ReconciliationResult::Resolved {
            from: "unknown".to_string(),
            to: "succeeded".to_string(),
        };
        assert_eq!(result1, result2);

        let result3 = ReconciliationResult::AlreadyResolved {
            current_status: "succeeded".to_string(),
        };
        assert_ne!(result1, result3);
    }

    #[test]
    fn test_psp_payment_status_variants() {
        let status = PspPaymentStatus::Succeeded;
        assert_eq!(status, PspPaymentStatus::Succeeded);

        let status = PspPaymentStatus::FailedRetry {
            code: "insufficient_funds".to_string(),
            message: "Insufficient funds".to_string(),
        };
        assert_eq!(
            status,
            PspPaymentStatus::FailedRetry {
                code: "insufficient_funds".to_string(),
                message: "Insufficient funds".to_string(),
            }
        );
    }
}
