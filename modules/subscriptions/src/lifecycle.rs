//! Subscription Lifecycle Guards and Transition Functions
//!
//! This module owns all lifecycle-critical mutations for subscription status.
//! All status updates MUST route through this module's functions.
//!
//! # State Machine
//! ```
//! ACTIVE ──> PAST_DUE ──> SUSPENDED
//!   ^                         |
//!   └─────────────────────────┘
//! ```
//!
//! # Critical Invariants
//! - Guards validate transitions only (zero side effects)
//! - Side effects occur AFTER guard approval
//! - Pattern: Guard → Mutation → Side Effect

use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Subscription Status Enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    /// Active subscription, billing normally
    Active,
    /// Payment failed, grace period active
    PastDue,
    /// Suspended due to non-payment
    Suspended,
}

impl SubscriptionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::PastDue => "past_due",
            Self::Suspended => "suspended",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, TransitionError> {
        match s {
            "active" => Ok(Self::Active),
            "past_due" => Ok(Self::PastDue),
            "suspended" => Ok(Self::Suspended),
            _ => Err(TransitionError::InvalidStatus {
                status: s.to_string(),
            }),
        }
    }
}

// ============================================================================
// Transition Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum TransitionError {
    #[error("Invalid status: {status}")]
    InvalidStatus { status: String },

    #[error("Illegal transition: {from} -> {to} (reason: {reason})")]
    IllegalTransition {
        from: String,
        to: String,
        reason: String,
    },

    #[error("Subscription not found: {subscription_id}")]
    SubscriptionNotFound { subscription_id: Uuid },

    #[error("Database error: {source}")]
    DatabaseError {
        #[from]
        source: sqlx::Error,
    },
}

// ============================================================================
// Transition Guard (ZERO SIDE EFFECTS)
// ============================================================================

/// Validates subscription status transitions.
///
/// # Critical Rules
/// - This function has ZERO side effects
/// - NO event emission
/// - NO HTTP calls
/// - NO ledger posts
/// - NO notification triggers
/// - NO external I/O
///
/// Side effects happen in the calling lifecycle function AFTER guard approval.
pub fn transition_guard(
    from: SubscriptionStatus,
    to: SubscriptionStatus,
    reason: &str,
) -> Result<(), TransitionError> {
    use SubscriptionStatus::*;

    let is_legal = match (from, to) {
        // ACTIVE transitions
        (Active, PastDue) => true,
        (Active, Active) => true, // idempotent

        // PAST_DUE transitions
        (PastDue, Suspended) => true,
        (PastDue, Active) => true, // payment recovered
        (PastDue, PastDue) => true, // idempotent

        // SUSPENDED transitions
        (Suspended, Active) => true, // reactivation
        (Suspended, Suspended) => true, // idempotent

        // All other transitions are illegal
        _ => false,
    };

    if is_legal {
        tracing::debug!(
            from = from.as_str(),
            to = to.as_str(),
            reason = reason,
            "Transition guard approved"
        );
        Ok(())
    } else {
        tracing::warn!(
            from = from.as_str(),
            to = to.as_str(),
            reason = reason,
            "Transition guard rejected: illegal transition"
        );
        Err(TransitionError::IllegalTransition {
            from: from.as_str().to_string(),
            to: to.as_str().to_string(),
            reason: reason.to_string(),
        })
    }
}

// ============================================================================
// Lifecycle Functions (Guard → Mutation → Side Effect)
// ============================================================================

/// Transition subscription to PAST_DUE status.
///
/// # Lifecycle Order
/// 1. Guard: Validate transition is legal
/// 2. Mutation: Update status in database
/// 3. Side Effect: (Future) Emit event, trigger notifications
pub async fn transition_to_past_due(
    subscription_id: Uuid,
    reason: &str,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    // Phase 16 bd-299f: Wrap in transaction for atomicity
    let mut tx = pool.begin().await?;

    // Fetch current status
    let current_status = fetch_current_status_tx(&mut tx, subscription_id).await?;

    // Guard: Validate transition (ZERO side effects)
    transition_guard(current_status, SubscriptionStatus::PastDue, reason)?;

    // Mutation: Update status (within transaction)
    update_status_tx(&mut tx, subscription_id, SubscriptionStatus::PastDue).await?;

    // Side Effect: Emit subscriptions.status.changed event atomically within the same TX
    let tenant_id: String = sqlx::query_scalar(
        "SELECT tenant_id FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&mut *tx)
    .await?;
    let status_payload = crate::models::SubscriptionStatusChangedPayload {
        subscription_id: subscription_id.to_string(),
        tenant_id: tenant_id.clone(),
        from_status: current_status.as_str().to_string(),
        to_status: SubscriptionStatus::PastDue.as_str().to_string(),
        reason: reason.to_string(),
    };
    let envelope = crate::envelope::create_subscriptions_envelope(
        uuid::Uuid::new_v4(),
        tenant_id,
        "subscriptions.status.changed".to_string(),
        None,
        None,
        "STATE_TRANSITION".to_string(),
        status_payload,
    );
    crate::outbox::enqueue_event_tx(&mut tx, "subscriptions.status.changed", &envelope).await?;

    tx.commit().await?;
    // - Emit subscriptions.status.changed event
    // - Trigger payment retry notification
    tracing::info!(
        subscription_id = %subscription_id,
        reason = reason,
        "Subscription transitioned to PAST_DUE"
    );

    Ok(())
}

/// Transition subscription to SUSPENDED status.
///
/// # Lifecycle Order
/// 1. Guard: Validate transition is legal
/// 2. Mutation: Update status in database
/// 3. Side Effect: (Future) Emit event, trigger notifications
pub async fn transition_to_suspended(
    subscription_id: Uuid,
    reason: &str,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    // Fetch current status
    let current_status = fetch_current_status(subscription_id, pool).await?;

    // Guard: Validate transition (ZERO side effects)
    transition_guard(current_status, SubscriptionStatus::Suspended, reason)?;

    // Mutation: Update status
    update_status(subscription_id, SubscriptionStatus::Suspended, pool).await?;

    // Side Effect: (Future implementation)
    // - Emit subscriptions.status.changed event
    // - Trigger suspension notification
    tracing::info!(
        subscription_id = %subscription_id,
        reason = reason,
        "Subscription transitioned to SUSPENDED"
    );

    Ok(())
}

/// Transition subscription to ACTIVE status (reactivation).
///
/// # Lifecycle Order
/// 1. Guard: Validate transition is legal
/// 2. Mutation: Update status in database
/// 3. Side Effect: (Future) Emit event, trigger notifications
pub async fn transition_to_active(
    subscription_id: Uuid,
    reason: &str,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    // Fetch current status
    let current_status = fetch_current_status(subscription_id, pool).await?;

    // Guard: Validate transition (ZERO side effects)
    transition_guard(current_status, SubscriptionStatus::Active, reason)?;

    // Mutation: Update status
    update_status(subscription_id, SubscriptionStatus::Active, pool).await?;

    // Side Effect: (Future implementation)
    // - Emit subscriptions.status.changed event
    // - Trigger reactivation notification
    tracing::info!(
        subscription_id = %subscription_id,
        reason = reason,
        "Subscription transitioned to ACTIVE"
    );

    Ok(())
}

// ============================================================================
// Internal Helpers (NOT exported)
// ============================================================================

/// Fetch current subscription status from database.
async fn fetch_current_status(
    subscription_id: Uuid,
    pool: &PgPool,
) -> Result<SubscriptionStatus, TransitionError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((status,)) => SubscriptionStatus::from_str(&status),
        None => Err(TransitionError::SubscriptionNotFound { subscription_id }),
    }
}

/// Update subscription status in database.
async fn update_status(
    subscription_id: Uuid,
    new_status: SubscriptionStatus,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    let rows_affected = sqlx::query(
        "UPDATE subscriptions SET status = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(new_status.as_str())
    .bind(subscription_id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(TransitionError::SubscriptionNotFound { subscription_id });
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
    fn test_guard_active_to_past_due() {
        let result = transition_guard(
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            "payment_failed",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_guard_past_due_to_suspended() {
        let result = transition_guard(
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Suspended,
            "grace_period_expired",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_guard_suspended_to_active() {
        let result = transition_guard(
            SubscriptionStatus::Suspended,
            SubscriptionStatus::Active,
            "payment_recovered",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_guard_past_due_to_active() {
        let result = transition_guard(
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Active,
            "payment_recovered",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_guard_idempotent_transitions() {
        // Same-state transitions should be allowed (idempotent)
        assert!(transition_guard(
            SubscriptionStatus::Active,
            SubscriptionStatus::Active,
            "idempotent"
        )
        .is_ok());

        assert!(transition_guard(
            SubscriptionStatus::PastDue,
            SubscriptionStatus::PastDue,
            "idempotent"
        )
        .is_ok());

        assert!(transition_guard(
            SubscriptionStatus::Suspended,
            SubscriptionStatus::Suspended,
            "idempotent"
        )
        .is_ok());
    }

    #[test]
    fn test_guard_illegal_active_to_suspended() {
        let result = transition_guard(
            SubscriptionStatus::Active,
            SubscriptionStatus::Suspended,
            "skip_past_due",
        );
        assert!(result.is_err());
        match result {
            Err(TransitionError::IllegalTransition { from, to, .. }) => {
                assert_eq!(from, "active");
                assert_eq!(to, "suspended");
            }
            _ => panic!("Expected IllegalTransition error"),
        }
    }

    #[test]
    fn test_guard_illegal_suspended_to_past_due() {
        let result = transition_guard(
            SubscriptionStatus::Suspended,
            SubscriptionStatus::PastDue,
            "backwards",
        );
        assert!(result.is_err());
        match result {
            Err(TransitionError::IllegalTransition { from, to, .. }) => {
                assert_eq!(from, "suspended");
                assert_eq!(to, "past_due");
            }
            _ => panic!("Expected IllegalTransition error"),
        }
    }

    #[test]
    fn test_status_roundtrip() {
        let statuses = [
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Suspended,
        ];

        for status in &statuses {
            let s = status.as_str();
            let parsed = SubscriptionStatus::from_str(s).unwrap();
            assert_eq!(*status, parsed);
        }
    }

    #[test]
    fn test_status_from_str_invalid() {
        let result = SubscriptionStatus::from_str("invalid");
        assert!(result.is_err());
        match result {
            Err(TransitionError::InvalidStatus { status }) => {
                assert_eq!(status, "invalid");
            }
            _ => panic!("Expected InvalidStatus error"),
        }
    }
}

// ============================================================================
// Transaction-Aware Helpers (Phase 16 bd-299f Atomicity Fix)
// ============================================================================

/// Fetch current subscription status from database (transaction-aware).
///
/// **Phase 16 Atomicity:** This version operates within an existing transaction
/// to ensure status read + mutation + event emission are atomic.
async fn fetch_current_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    subscription_id: Uuid,
) -> Result<SubscriptionStatus, TransitionError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_optional(&mut **tx)
    .await?;

    match row {
        Some((status,)) => SubscriptionStatus::from_str(&status),
        None => Err(TransitionError::SubscriptionNotFound { subscription_id }),
    }
}

/// Update subscription status in database (transaction-aware).
///
/// **Phase 16 Atomicity:** This version operates within an existing transaction
/// to ensure mutation + event emission commit atomically.
async fn update_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    subscription_id: Uuid,
    new_status: SubscriptionStatus,
) -> Result<(), TransitionError> {
    let rows_affected = sqlx::query(
        "UPDATE subscriptions SET status = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind(new_status.as_str())
    .bind(subscription_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(TransitionError::SubscriptionNotFound { subscription_id });
    }

    Ok(())
}
