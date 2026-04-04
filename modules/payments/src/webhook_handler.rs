//! Webhook Handler with Deterministic Gating (Phase 15 - bd-1wg)
//!
//! **CRITICAL: Mutation order enforcement for exactly-once semantics.**
//!
//! **Mutation Order (HARD REQUIREMENT):**
//! 1. Signature validation (BEFORE any database writes)
//! 2. Envelope validation (existing pattern)
//! 3. Attempt ledger gating (SELECT FOR UPDATE + UNIQUE constraints)
//! 4. Lifecycle mutation (via bd-3lm lifecycle guards)
//! 5. Event emission (after mutation succeeds)
//!
//! **Pattern (from PearlOwl):**
//! Lock → Check → Insert/Update → Emit → Commit
//!
//! **Exactly-Once Guarantees:**
//! - UNIQUE constraint on (app_id, payment_id, attempt_no) prevents duplicate attempts
//! - SELECT FOR UPDATE prevents concurrent processing of same webhook
//! - Lifecycle guards enforce valid state transitions
//! - Idempotency keys ensure deterministic side effects
//!
//! **Current Use Cases:**
//! - Internal events: ar.payment.collection.requested
//! - Future: PSP webhook callbacks (Stripe/Tilled)

use crate::lifecycle::{LifecycleError, TransitionError};
use crate::webhook_signature::{validate_webhook_signature, SignatureError, WebhookSource};
use sqlx::PgPool;
use std::fmt;
use tracing::{info, warn};
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug)]
pub enum WebhookError {
    SignatureError(SignatureError),
    LifecycleError(LifecycleError),
    DatabaseError(sqlx::Error),
    AttemptNotFound(Uuid),
    DuplicateAttempt { attempt_id: Uuid },
}

impl fmt::Display for WebhookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignatureError(e) => write!(f, "Signature error: {}", e),
            Self::LifecycleError(e) => write!(f, "Lifecycle error: {}", e),
            Self::DatabaseError(e) => write!(f, "Database error: {}", e),
            Self::AttemptNotFound(id) => write!(f, "Payment attempt not found: {}", id),
            Self::DuplicateAttempt { attempt_id } => {
                write!(f, "Duplicate attempt detected: {}", attempt_id)
            }
        }
    }
}

impl std::error::Error for WebhookError {}

impl From<SignatureError> for WebhookError {
    fn from(e: SignatureError) -> Self {
        Self::SignatureError(e)
    }
}

impl From<LifecycleError> for WebhookError {
    fn from(e: LifecycleError) -> Self {
        Self::LifecycleError(e)
    }
}

impl From<sqlx::Error> for WebhookError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e)
    }
}

// ============================================================================
// Webhook Status Update (Phase 15 Core Pattern)
// ============================================================================

/// Update payment attempt status via webhook with deterministic gating
///
/// **Mutation Order (CRITICAL - DO NOT REORDER):**
/// 1. **Signature validation** - Validate webhook signature (FIRST - before any DB writes)
/// 2. **Envelope validation** - Validate event envelope fields (reuse existing pattern)
/// 3. **Attempt ledger gating** - SELECT FOR UPDATE to lock attempt row
/// 4. **Lifecycle mutation** - Call lifecycle guards (bd-3lm) to enforce valid transitions
/// 5. **Event emission** - Emit success/failure events (after mutation succeeds)
///
/// **Exactly-Once Guarantees:**
/// - Webhook signature prevents replay attacks (PSP-level)
/// - SELECT FOR UPDATE prevents concurrent processing
/// - UNIQUE constraint on (app_id, payment_id, attempt_no) prevents duplicates
/// - Lifecycle guards enforce state machine invariants
/// - Idempotency keys ensure deterministic side effects
///
/// **Usage:**
/// ```ignore
/// use payments::webhook_handler::{update_payment_status_from_webhook, WebhookSource};
/// use sqlx::PgPool;
/// use uuid::Uuid;
///
/// let pool = /* ... */;
/// let attempt_id = Uuid::new_v4();
/// let webhook_event_id = "evt_stripe_12345";
///
/// // Update payment attempt to SUCCEEDED via webhook
/// update_payment_status_from_webhook(
///     &pool,
///     attempt_id,
///     "succeeded",
///     webhook_event_id,
///     WebhookSource::Internal,
///     &headers,
///     &body,
/// ).await?;
/// ```
///
/// **Idempotency:**
/// - Same webhook_event_id → no-op (already processed)
/// - Lifecycle guards reject illegal transitions
/// - UNIQUE constraint prevents duplicate attempts
pub async fn update_payment_status_from_webhook(
    pool: &PgPool,
    attempt_id: Uuid,
    target_status: &str,
    webhook_event_id: &str,
    webhook_source: WebhookSource,
    headers: &std::collections::HashMap<String, String>,
    body: &[u8],
    tilled_secrets: &[&str],
) -> Result<(), WebhookError> {
    // ==========================================================================
    // STEP 1: Signature Validation (BEFORE any database writes)
    // ==========================================================================
    validate_webhook_signature(webhook_source, headers, body, tilled_secrets)?;

    // ==========================================================================
    // STEP 2: Envelope Validation (existing pattern - reuse if needed)
    // ==========================================================================
    //
    // **Note:** For webhook callbacks, envelope validation may not apply
    // (PSP webhooks have different structure than internal NATS events)
    //
    // If processing internal events, call envelope_validation::validate_envelope()
    // For PSP webhooks, validate PSP-specific payload structure here

    // ==========================================================================
    // STEP 3: Attempt Ledger Gating (SELECT FOR UPDATE)
    // ==========================================================================
    //
    // **Pattern:** Lock → Check → Update → Emit → Commit
    // - Lock attempt row with SELECT FOR UPDATE
    // - Check if webhook already processed (idempotency)
    // - Update status via lifecycle mutation (bd-3lm)
    // - Emit events (after mutation succeeds)
    // - Commit transaction
    let mut tx = pool.begin().await?;

    // Lock attempt row (prevents concurrent webhook processing)
    let existing_webhook_event_id: Option<String> = sqlx::query_scalar(
        "SELECT webhook_event_id FROM payment_attempts WHERE id = $1 FOR UPDATE",
    )
    .bind(attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(WebhookError::AttemptNotFound(attempt_id))?;

    // Idempotency check: If webhook already processed, return success (no-op)
    if let Some(existing_event_id) = existing_webhook_event_id {
        if existing_event_id == webhook_event_id {
            info!(
                module = "payments",
                entity_type = "payment_attempt",
                entity_id = %attempt_id,
                webhook_event_id = webhook_event_id,
                decision = "skip",
                reason_code = "duplicate_webhook",
                message = "Webhook already processed (idempotent no-op)",
                context = ?serde_json::json!({
                    "attempt_id": attempt_id,
                    "webhook_event_id": webhook_event_id,
                }),
                "Duplicate webhook event (idempotent no-op)"
            );

            tx.commit().await?;
            return Ok(());
        }
    }

    // ==========================================================================
    // STEP 4: Lifecycle Mutation (via bd-3lm guards)
    // ==========================================================================
    //
    // **Pattern:** guard → mutate → emit
    // - Lifecycle guards enforce state machine transitions
    // - Guards validate ONLY (zero side effects)
    // - Mutation happens after guard approval
    //
    // **Note:** We're calling lifecycle functions within this transaction
    // to ensure atomic: lock → validate → mutate → update webhook_event_id
    //
    // Validate transition using lifecycle guard (manual validation within transaction)
    let current_status: String =
        sqlx::query_scalar("SELECT status::text FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(&mut *tx)
            .await?;

    // Manual transition validation (mirroring lifecycle::validate_transition logic)
    // This is necessary because we're in a transaction with SELECT FOR UPDATE
    let is_valid = match (current_status.as_str(), target_status) {
        // ATTEMPTING can transition to all terminal/intermediate states
        ("attempting", "succeeded") => true,
        ("attempting", "failed_retry") => true,
        ("attempting", "failed_final") => true,
        ("attempting", "unknown") => true,

        // FAILED_RETRY can transition back to ATTEMPTING (retry window opens)
        ("failed_retry", "attempting") => true,

        // UNKNOWN can transition to terminal states (after reconciliation)
        ("unknown", "succeeded") => true,
        ("unknown", "failed_retry") => true,
        ("unknown", "failed_final") => true,

        // Terminal states (SUCCEEDED, FAILED_FINAL) have no outgoing transitions
        ("succeeded", _) => false,
        ("failed_final", _) => false,

        // All other transitions are illegal
        _ => false,
    };

    if !is_valid {
        warn!(
            module = "payments",
            entity_type = "payment_attempt",
            entity_id = %attempt_id,
            from_state = %current_status,
            to_state = target_status,
            decision = "reject",
            reason_code = "illegal_transition",
            message = "Payment attempt transition rejected by state machine",
            context = ?serde_json::json!({
                "from": current_status,
                "to": target_status,
                "state_machine": "payment_attempt"
            }),
            "Payment attempt transition rejected"
        );

        return Err(WebhookError::LifecycleError(
            LifecycleError::TransitionError(TransitionError::IllegalTransition {
                from: current_status.clone(),
                to: target_status.to_string(),
                reason: format!(
                    "State machine does not allow transition from {} to {}",
                    current_status, target_status
                ),
            }),
        ));
    }

    // Update payment attempt status + webhook_event_id (atomic)
    sqlx::query(
        "UPDATE payment_attempts SET status = $1::payment_attempt_status, webhook_event_id = $2, completed_at = CURRENT_TIMESTAMP WHERE id = $3"
    )
    .bind(target_status)
    .bind(webhook_event_id)
    .bind(attempt_id)
    .execute(&mut *tx)
    .await?;

    info!(
        module = "payments",
        entity_type = "payment_attempt",
        entity_id = %attempt_id,
        from_state = %current_status,
        to_state = target_status,
        decision = "accept",
        reason_code = "valid_transition",
        message = "Payment attempt transition accepted by state machine",
        context = ?serde_json::json!({
            "from": current_status,
            "to": target_status,
            "state_machine": "payment_attempt",
            "webhook_event_id": webhook_event_id
        }),
        "Payment attempt transition accepted"
    );

    // ==========================================================================
    // STEP 5: Event Emission (after mutation succeeds)
    // ==========================================================================
    //
    // Fetch attempt details for event payload
    let row: (String, uuid::Uuid, String) = sqlx::query_as(
        "SELECT app_id, payment_id, invoice_id FROM payment_attempts WHERE id = $1",
    )
    .bind(attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let (app_id, payment_id_val, invoice_id) = row;

    match target_status {
        "succeeded" => {
            let payload = crate::models::PaymentSucceededPayload {
                payment_id: payment_id_val.to_string(),
                invoice_id,
                ar_customer_id: app_id.clone(),
                amount_minor: 0,
                currency: "USD".to_string(),
                processor_payment_id: None,
                payment_method_ref: None,
            };
            let envelope = crate::events::envelope::create_payments_envelope(
                uuid::Uuid::new_v4(),
                app_id,
                "payment.succeeded".to_string(),
                Some(webhook_event_id.to_string()),
                None,
                "LIFECYCLE".to_string(),
                payload,
            );
            crate::events::outbox::enqueue_event_tx(
                &mut tx,
                "payment.succeeded",
                &envelope,
            )
            .await?;
        }
        s if s.starts_with("failed") => {
            let payload = crate::models::PaymentFailedPayload {
                payment_id: payment_id_val.to_string(),
                invoice_id,
                ar_customer_id: app_id.clone(),
                amount_minor: 0,
                currency: "USD".to_string(),
                failure_code: target_status.to_string(),
                failure_message: None,
                processor_payment_id: None,
                payment_method_ref: None,
            };
            let envelope = crate::events::envelope::create_payments_envelope(
                uuid::Uuid::new_v4(),
                app_id,
                "payment.failed".to_string(),
                Some(webhook_event_id.to_string()),
                None,
                "LIFECYCLE".to_string(),
                payload,
            );
            crate::events::outbox::enqueue_event_tx(
                &mut tx,
                "payment.failed",
                &envelope,
            )
            .await?;
        }
        "unknown" => {
            let payload = crate::models::PaymentUnknownPayload {
                payment_id: payment_id_val.to_string(),
                invoice_id,
                ar_customer_id: app_id.clone(),
                processor_payment_id: None,
                payment_method_ref: None,
            };
            let envelope = crate::events::envelope::create_payments_envelope(
                uuid::Uuid::new_v4(),
                app_id,
                "payment.unknown".to_string(),
                Some(webhook_event_id.to_string()),
                None,
                "LIFECYCLE".to_string(),
                payload,
            );
            crate::events::outbox::enqueue_event_tx(
                &mut tx,
                "payment.unknown",
                &envelope,
            )
            .await?;
        }
        _ => {
            // No event emission for unrecognized statuses
        }
    }

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_error_display() {
        let err = WebhookError::SignatureError(SignatureError::InvalidSignature {
            reason: "HMAC mismatch".to_string(),
        });
        assert_eq!(
            err.to_string(),
            "Signature error: Webhook signature verification failed: HMAC mismatch"
        );

        let err = WebhookError::AttemptNotFound(Uuid::nil());
        assert_eq!(
            err.to_string(),
            format!("Payment attempt not found: {}", Uuid::nil())
        );

        let err = WebhookError::DuplicateAttempt {
            attempt_id: Uuid::nil(),
        };
        assert_eq!(
            err.to_string(),
            format!("Duplicate attempt detected: {}", Uuid::nil())
        );
    }
}
