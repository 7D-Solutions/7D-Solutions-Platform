use super::types::WoStatus;
use thiserror::Error;

/// Error returned when a work order status transition is not allowed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("cannot transition from terminal status '{from}'")]
    FromTerminal { from: WoStatus },

    #[error("transition from '{from}' to '{to}' is not allowed")]
    NotAllowed { from: WoStatus, to: WoStatus },

    #[error("transition to same status '{status}' is a no-op")]
    NoOp { status: WoStatus },
}

/// Returns the set of statuses reachable from `from`.
///
/// ```text
/// draft → awaiting_approval, scheduled, cancelled
/// awaiting_approval → scheduled, cancelled
/// scheduled → in_progress, cancelled
/// in_progress → on_hold, completed, cancelled
/// on_hold → in_progress, cancelled
/// completed → closed
/// closed → (terminal)
/// cancelled → (terminal)
/// ```
pub fn allowed_transitions(from: WoStatus) -> &'static [WoStatus] {
    match from {
        WoStatus::Draft => &[
            WoStatus::AwaitingApproval,
            WoStatus::Scheduled,
            WoStatus::Cancelled,
        ],
        WoStatus::AwaitingApproval => &[WoStatus::Scheduled, WoStatus::Cancelled],
        WoStatus::Scheduled => &[WoStatus::InProgress, WoStatus::Cancelled],
        WoStatus::InProgress => &[WoStatus::OnHold, WoStatus::Completed, WoStatus::Cancelled],
        WoStatus::OnHold => &[WoStatus::InProgress, WoStatus::Cancelled],
        WoStatus::Completed => &[WoStatus::Closed],
        WoStatus::Closed => &[],
        WoStatus::Cancelled => &[],
    }
}

/// Validate that transitioning from `from` to `to` is structurally allowed.
///
/// This checks the state machine graph only. Field-level guards (e.g.
/// completed requires `completed_at`) are enforced separately in `guards.rs`.
pub fn validate_transition(from: WoStatus, to: WoStatus) -> Result<(), TransitionError> {
    if from == to {
        return Err(TransitionError::NoOp { status: from });
    }

    if from.is_terminal() {
        return Err(TransitionError::FromTerminal { from });
    }

    if allowed_transitions(from).contains(&to) {
        Ok(())
    } else {
        Err(TransitionError::NotAllowed { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Allowed transitions ──────────────────────────────────────────

    #[test]
    fn draft_to_awaiting_approval() {
        assert!(validate_transition(WoStatus::Draft, WoStatus::AwaitingApproval).is_ok());
    }

    #[test]
    fn draft_to_scheduled() {
        assert!(validate_transition(WoStatus::Draft, WoStatus::Scheduled).is_ok());
    }

    #[test]
    fn draft_to_cancelled() {
        assert!(validate_transition(WoStatus::Draft, WoStatus::Cancelled).is_ok());
    }

    #[test]
    fn awaiting_approval_to_scheduled() {
        assert!(validate_transition(WoStatus::AwaitingApproval, WoStatus::Scheduled).is_ok());
    }

    #[test]
    fn awaiting_approval_to_cancelled() {
        assert!(validate_transition(WoStatus::AwaitingApproval, WoStatus::Cancelled).is_ok());
    }

    #[test]
    fn scheduled_to_in_progress() {
        assert!(validate_transition(WoStatus::Scheduled, WoStatus::InProgress).is_ok());
    }

    #[test]
    fn scheduled_to_cancelled() {
        assert!(validate_transition(WoStatus::Scheduled, WoStatus::Cancelled).is_ok());
    }

    #[test]
    fn in_progress_to_on_hold() {
        assert!(validate_transition(WoStatus::InProgress, WoStatus::OnHold).is_ok());
    }

    #[test]
    fn in_progress_to_completed() {
        assert!(validate_transition(WoStatus::InProgress, WoStatus::Completed).is_ok());
    }

    #[test]
    fn in_progress_to_cancelled() {
        assert!(validate_transition(WoStatus::InProgress, WoStatus::Cancelled).is_ok());
    }

    #[test]
    fn on_hold_to_in_progress() {
        assert!(validate_transition(WoStatus::OnHold, WoStatus::InProgress).is_ok());
    }

    #[test]
    fn on_hold_to_cancelled() {
        assert!(validate_transition(WoStatus::OnHold, WoStatus::Cancelled).is_ok());
    }

    #[test]
    fn completed_to_closed() {
        assert!(validate_transition(WoStatus::Completed, WoStatus::Closed).is_ok());
    }

    // ── Forbidden transitions ────────────────────────────────────────

    #[test]
    fn closed_is_terminal() {
        let err = validate_transition(WoStatus::Closed, WoStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::FromTerminal {
                from: WoStatus::Closed
            }
        );
    }

    #[test]
    fn cancelled_is_terminal() {
        let err = validate_transition(WoStatus::Cancelled, WoStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::FromTerminal {
                from: WoStatus::Cancelled
            }
        );
    }

    #[test]
    fn no_op_same_status() {
        let err = validate_transition(WoStatus::Draft, WoStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NoOp {
                status: WoStatus::Draft
            }
        );
    }

    #[test]
    fn draft_cannot_go_to_in_progress() {
        let err = validate_transition(WoStatus::Draft, WoStatus::InProgress).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::Draft,
                to: WoStatus::InProgress
            }
        );
    }

    #[test]
    fn draft_cannot_go_to_completed() {
        let err = validate_transition(WoStatus::Draft, WoStatus::Completed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::Draft,
                to: WoStatus::Completed
            }
        );
    }

    #[test]
    fn draft_cannot_go_to_closed() {
        let err = validate_transition(WoStatus::Draft, WoStatus::Closed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::Draft,
                to: WoStatus::Closed
            }
        );
    }

    #[test]
    fn scheduled_cannot_go_to_completed() {
        let err = validate_transition(WoStatus::Scheduled, WoStatus::Completed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::Scheduled,
                to: WoStatus::Completed
            }
        );
    }

    #[test]
    fn on_hold_cannot_go_to_completed() {
        let err = validate_transition(WoStatus::OnHold, WoStatus::Completed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::OnHold,
                to: WoStatus::Completed
            }
        );
    }

    #[test]
    fn completed_cannot_go_to_in_progress() {
        let err = validate_transition(WoStatus::Completed, WoStatus::InProgress).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::Completed,
                to: WoStatus::InProgress
            }
        );
    }

    #[test]
    fn completed_cannot_go_to_cancelled() {
        let err = validate_transition(WoStatus::Completed, WoStatus::Cancelled).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NotAllowed {
                from: WoStatus::Completed,
                to: WoStatus::Cancelled
            }
        );
    }

    // ── Exhaustive: every pair is either allowed or rejected ─────────

    #[test]
    fn all_transitions_are_deterministic() {
        let all = [
            WoStatus::Draft,
            WoStatus::AwaitingApproval,
            WoStatus::Scheduled,
            WoStatus::InProgress,
            WoStatus::OnHold,
            WoStatus::Completed,
            WoStatus::Closed,
            WoStatus::Cancelled,
        ];

        for from in &all {
            for to in &all {
                let result = validate_transition(*from, *to);
                // Every combination must produce a definite Ok or Err — no panics.
                let _ = result;
            }
        }
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        assert!(allowed_transitions(WoStatus::Closed).is_empty());
        assert!(allowed_transitions(WoStatus::Cancelled).is_empty());
    }

    #[test]
    fn every_non_terminal_can_be_cancelled() {
        let non_terminal = [
            WoStatus::Draft,
            WoStatus::AwaitingApproval,
            WoStatus::Scheduled,
            WoStatus::InProgress,
            WoStatus::OnHold,
        ];
        for status in &non_terminal {
            assert!(
                validate_transition(*status, WoStatus::Cancelled).is_ok(),
                "{} should be cancellable",
                status
            );
        }
        // completed cannot be cancelled — must go through closed
        assert!(validate_transition(WoStatus::Completed, WoStatus::Cancelled).is_err());
    }
}
