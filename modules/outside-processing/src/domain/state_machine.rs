use super::models::{OpError, OpOrderStatus, ReviewOutcome};

/// Validate and return the next status for an issue transition.
pub fn transition_issue(current: &str) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(OpOrderStatus::Draft) => Ok(OpOrderStatus::Issued),
        Some(s) if s.is_terminal() => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "issued".to_string(),
        }),
        _ => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "issued".to_string(),
        }),
    }
}

/// Status after recording a ship event (first ship → shipped_to_vendor; rework from at_vendor → shipped_to_vendor).
pub fn transition_on_ship_event(current: &str) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(OpOrderStatus::Issued) | Some(OpOrderStatus::AtVendor) => {
            Ok(OpOrderStatus::ShippedToVendor)
        }
        _ => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "shipped_to_vendor".to_string(),
        }),
    }
}

/// Status after recording a return event (first return → returned; subsequent returns stay returned).
pub fn transition_on_return_event(current: &str) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(OpOrderStatus::ShippedToVendor)
        | Some(OpOrderStatus::AtVendor)
        | Some(OpOrderStatus::Returned) => Ok(OpOrderStatus::Returned),
        _ => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "returned".to_string(),
        }),
    }
}

/// Status after recording a review (first review → review_in_progress).
pub fn transition_on_review_created(current: &str) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(OpOrderStatus::Returned) | Some(OpOrderStatus::ReviewInProgress) => {
            Ok(OpOrderStatus::ReviewInProgress)
        }
        _ => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "review_in_progress".to_string(),
        }),
    }
}

/// Status after a review outcome is recorded.
/// - accepted/conditional → closed
/// - rejected + rework → at_vendor
/// - rejected + no rework → review_in_progress (awaiting next disposition)
pub fn transition_on_review_outcome(
    current: &str,
    outcome: ReviewOutcome,
    rework: bool,
) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(OpOrderStatus::ReviewInProgress) => match outcome {
            ReviewOutcome::Accepted | ReviewOutcome::Conditional => Ok(OpOrderStatus::Closed),
            ReviewOutcome::Rejected if rework => Ok(OpOrderStatus::AtVendor),
            ReviewOutcome::Rejected => Ok(OpOrderStatus::ReviewInProgress),
        },
        _ => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "review outcome transition".to_string(),
        }),
    }
}

/// Cancel from any non-terminal state.
pub fn transition_cancel(current: &str) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(s) if s.is_terminal() => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "cancelled".to_string(),
        }),
        Some(_) => Ok(OpOrderStatus::Cancelled),
        None => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "cancelled".to_string(),
        }),
    }
}

/// Close from review_in_progress (manual close by operator).
pub fn transition_close(current: &str) -> Result<OpOrderStatus, OpError> {
    match OpOrderStatus::from_str(current) {
        Some(OpOrderStatus::ReviewInProgress) => Ok(OpOrderStatus::Closed),
        Some(s) if s.is_terminal() => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "closed".to_string(),
        }),
        _ => Err(OpError::InvalidTransition {
            from: current.to_string(),
            to: "closed".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_to_issued() {
        assert_eq!(transition_issue("draft").unwrap(), OpOrderStatus::Issued);
    }

    #[test]
    fn cannot_issue_non_draft() {
        assert!(transition_issue("issued").is_err());
        assert!(transition_issue("closed").is_err());
        assert!(transition_issue("cancelled").is_err());
    }

    #[test]
    fn ship_from_issued_goes_to_shipped_to_vendor() {
        assert_eq!(
            transition_on_ship_event("issued").unwrap(),
            OpOrderStatus::ShippedToVendor
        );
    }

    #[test]
    fn ship_from_at_vendor_goes_to_shipped_to_vendor_rework() {
        assert_eq!(
            transition_on_ship_event("at_vendor").unwrap(),
            OpOrderStatus::ShippedToVendor
        );
    }

    #[test]
    fn return_event_transitions() {
        assert_eq!(
            transition_on_return_event("shipped_to_vendor").unwrap(),
            OpOrderStatus::Returned
        );
        assert_eq!(
            transition_on_return_event("returned").unwrap(),
            OpOrderStatus::Returned
        );
    }

    #[test]
    fn review_accepted_closes_order() {
        assert_eq!(
            transition_on_review_outcome("review_in_progress", ReviewOutcome::Accepted, false)
                .unwrap(),
            OpOrderStatus::Closed
        );
    }

    #[test]
    fn review_rejected_rework_returns_to_at_vendor() {
        assert_eq!(
            transition_on_review_outcome("review_in_progress", ReviewOutcome::Rejected, true)
                .unwrap(),
            OpOrderStatus::AtVendor
        );
    }

    #[test]
    fn review_rejected_no_rework_stays_review_in_progress() {
        assert_eq!(
            transition_on_review_outcome("review_in_progress", ReviewOutcome::Rejected, false)
                .unwrap(),
            OpOrderStatus::ReviewInProgress
        );
    }

    #[test]
    fn cancel_from_terminal_fails() {
        assert!(transition_cancel("closed").is_err());
        assert!(transition_cancel("cancelled").is_err());
    }

    #[test]
    fn cancel_from_any_non_terminal_succeeds() {
        for status in &[
            "draft",
            "issued",
            "shipped_to_vendor",
            "at_vendor",
            "returned",
            "review_in_progress",
        ] {
            assert_eq!(transition_cancel(status).unwrap(), OpOrderStatus::Cancelled);
        }
    }
}
