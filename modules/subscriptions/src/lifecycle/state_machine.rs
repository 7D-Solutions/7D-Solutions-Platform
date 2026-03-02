//! Subscription state machine: status enum, transition guard, and error types.
//!
//! Pure logic — no database access. Agents modifying suspension triggers
//! or transition rules should edit this file only.

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
        (Active, Suspended) => true, // dunning terminal escalation
        (Active, Active) => true,    // idempotent

        // PAST_DUE transitions
        (PastDue, Suspended) => true,
        (PastDue, Active) => true,  // payment recovered
        (PastDue, PastDue) => true, // idempotent

        // SUSPENDED transitions
        (Suspended, Active) => true,    // reactivation
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
    fn test_guard_active_to_suspended() {
        // Active → Suspended is legal (dunning terminal escalation)
        let result = transition_guard(
            SubscriptionStatus::Active,
            SubscriptionStatus::Suspended,
            "dunning_suspension",
        );
        assert!(result.is_ok());
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
