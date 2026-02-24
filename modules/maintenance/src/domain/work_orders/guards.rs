use super::types::WoStatus;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// Error returned when a field-level guard fails during a status transition.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuardError {
    #[error("completed_at is required when transitioning to completed")]
    MissingCompletedAt,

    #[error("downtime_minutes is required when transitioning to completed")]
    MissingDowntimeMinutes,

    #[error("closed_at is required when transitioning to closed")]
    MissingClosedAt,
}

/// Context provided to field-level guards during a transition.
/// The caller populates whichever fields are relevant.
pub struct TransitionContext {
    pub completed_at: Option<DateTime<Utc>>,
    pub downtime_minutes: Option<i32>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// Validate field-level requirements for transitioning to `completed`.
///
/// The spec requires:
/// - `completed_at` must be set
/// - `downtime_minutes` must be set
pub fn validate_completion_fields(ctx: &TransitionContext) -> Result<(), GuardError> {
    if ctx.completed_at.is_none() {
        return Err(GuardError::MissingCompletedAt);
    }
    if ctx.downtime_minutes.is_none() {
        return Err(GuardError::MissingDowntimeMinutes);
    }
    Ok(())
}

/// Validate field-level requirements for transitioning to `closed`.
///
/// Requires `closed_at` to be set so the close timestamp is recorded.
pub fn validate_close_fields(ctx: &TransitionContext) -> Result<(), GuardError> {
    if ctx.closed_at.is_none() {
        return Err(GuardError::MissingClosedAt);
    }
    Ok(())
}

/// Run all applicable field-level guards for the given target status.
///
/// Returns `Ok(())` if no guards apply or all guards pass.
pub fn run_guards(to: WoStatus, ctx: &TransitionContext) -> Result<(), GuardError> {
    match to {
        WoStatus::Completed => validate_completion_fields(ctx),
        WoStatus::Closed => validate_close_fields(ctx),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn empty_ctx() -> TransitionContext {
        TransitionContext {
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
        }
    }

    // ── Completion guards ────────────────────────────────────────────

    #[test]
    fn completion_requires_completed_at() {
        let ctx = TransitionContext {
            downtime_minutes: Some(30),
            ..empty_ctx()
        };
        assert_eq!(
            validate_completion_fields(&ctx),
            Err(GuardError::MissingCompletedAt)
        );
    }

    #[test]
    fn completion_requires_downtime_minutes() {
        let ctx = TransitionContext {
            completed_at: Some(Utc::now()),
            ..empty_ctx()
        };
        assert_eq!(
            validate_completion_fields(&ctx),
            Err(GuardError::MissingDowntimeMinutes)
        );
    }

    #[test]
    fn completion_passes_with_both_fields() {
        let ctx = TransitionContext {
            completed_at: Some(Utc::now()),
            downtime_minutes: Some(0),
            ..empty_ctx()
        };
        assert!(validate_completion_fields(&ctx).is_ok());
    }

    #[test]
    fn completion_accepts_zero_downtime() {
        let ctx = TransitionContext {
            completed_at: Some(Utc::now()),
            downtime_minutes: Some(0),
            ..empty_ctx()
        };
        assert!(validate_completion_fields(&ctx).is_ok());
    }

    // ── Close guards ─────────────────────────────────────────────────

    #[test]
    fn close_requires_closed_at() {
        let ctx = empty_ctx();
        assert_eq!(
            validate_close_fields(&ctx),
            Err(GuardError::MissingClosedAt)
        );
    }

    #[test]
    fn close_passes_with_closed_at() {
        let ctx = TransitionContext {
            closed_at: Some(Utc::now()),
            ..empty_ctx()
        };
        assert!(validate_close_fields(&ctx).is_ok());
    }

    // ── run_guards dispatch ──────────────────────────────────────────

    #[test]
    fn run_guards_dispatches_to_completion() {
        let ctx = empty_ctx();
        assert_eq!(
            run_guards(WoStatus::Completed, &ctx),
            Err(GuardError::MissingCompletedAt)
        );
    }

    #[test]
    fn run_guards_dispatches_to_close() {
        let ctx = empty_ctx();
        assert_eq!(
            run_guards(WoStatus::Closed, &ctx),
            Err(GuardError::MissingClosedAt)
        );
    }

    #[test]
    fn run_guards_passes_for_non_guarded_status() {
        let ctx = empty_ctx();
        assert!(run_guards(WoStatus::Scheduled, &ctx).is_ok());
        assert!(run_guards(WoStatus::InProgress, &ctx).is_ok());
        assert!(run_guards(WoStatus::OnHold, &ctx).is_ok());
        assert!(run_guards(WoStatus::Cancelled, &ctx).is_ok());
        assert!(run_guards(WoStatus::Draft, &ctx).is_ok());
        assert!(run_guards(WoStatus::AwaitingApproval, &ctx).is_ok());
    }
}
