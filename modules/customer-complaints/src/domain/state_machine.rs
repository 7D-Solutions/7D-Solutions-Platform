use super::models::{ComplaintError, ComplaintStatus};

/// intake → triaged: requires category + severity + assignee set (caller validates before calling)
pub fn transition_triage(current: &str) -> Result<ComplaintStatus, ComplaintError> {
    match ComplaintStatus::from_str(current) {
        Some(ComplaintStatus::Intake) => Ok(ComplaintStatus::Triaged),
        Some(s) if s.is_terminal() => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "triaged".to_string(),
            reason: "complaint is in a terminal state".to_string(),
        }),
        _ => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "triaged".to_string(),
            reason: "only intake complaints can be triaged".to_string(),
        }),
    }
}

/// triaged → investigating
pub fn transition_start_investigation(current: &str) -> Result<ComplaintStatus, ComplaintError> {
    match ComplaintStatus::from_str(current) {
        Some(ComplaintStatus::Triaged) => Ok(ComplaintStatus::Investigating),
        Some(s) if s.is_terminal() => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "investigating".to_string(),
            reason: "complaint is in a terminal state".to_string(),
        }),
        _ => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "investigating".to_string(),
            reason: "complaint must be triaged before investigation can start".to_string(),
        }),
    }
}

/// investigating → responded (caller must verify customer_communication activity exists)
pub fn transition_respond(current: &str) -> Result<ComplaintStatus, ComplaintError> {
    match ComplaintStatus::from_str(current) {
        Some(ComplaintStatus::Investigating) => Ok(ComplaintStatus::Responded),
        Some(s) if s.is_terminal() => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "responded".to_string(),
            reason: "complaint is in a terminal state".to_string(),
        }),
        _ => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "responded".to_string(),
            reason: "complaint must be investigating before it can be responded".to_string(),
        }),
    }
}

/// responded → closed (caller must verify complaint_resolution record exists)
pub fn transition_close(current: &str) -> Result<ComplaintStatus, ComplaintError> {
    match ComplaintStatus::from_str(current) {
        Some(ComplaintStatus::Responded) => Ok(ComplaintStatus::Closed),
        Some(s) if s.is_terminal() => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "closed".to_string(),
            reason: "complaint is in a terminal state".to_string(),
        }),
        _ => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "closed".to_string(),
            reason: "complaint must be responded before it can be closed".to_string(),
        }),
    }
}

/// * → cancelled (from any non-terminal state)
pub fn transition_cancel(current: &str) -> Result<ComplaintStatus, ComplaintError> {
    match ComplaintStatus::from_str(current) {
        Some(s) if s.is_terminal() => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "cancelled".to_string(),
            reason: "complaint is already in a terminal state".to_string(),
        }),
        Some(_) => Ok(ComplaintStatus::Cancelled),
        None => Err(ComplaintError::InvalidTransition {
            from: current.to_string(),
            to: "cancelled".to_string(),
            reason: "unknown complaint status".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intake_to_triaged() {
        assert_eq!(
            transition_triage("intake").unwrap(),
            ComplaintStatus::Triaged
        );
    }

    #[test]
    fn cannot_triage_non_intake() {
        assert!(transition_triage("triaged").is_err());
        assert!(transition_triage("investigating").is_err());
        assert!(transition_triage("closed").is_err());
        assert!(transition_triage("cancelled").is_err());
    }

    #[test]
    fn triaged_to_investigating() {
        assert_eq!(
            transition_start_investigation("triaged").unwrap(),
            ComplaintStatus::Investigating
        );
    }

    #[test]
    fn cannot_start_investigation_from_non_triaged() {
        assert!(transition_start_investigation("intake").is_err());
        assert!(transition_start_investigation("investigating").is_err());
        assert!(transition_start_investigation("closed").is_err());
    }

    #[test]
    fn investigating_to_responded() {
        assert_eq!(
            transition_respond("investigating").unwrap(),
            ComplaintStatus::Responded
        );
    }

    #[test]
    fn cannot_respond_from_non_investigating() {
        assert!(transition_respond("intake").is_err());
        assert!(transition_respond("triaged").is_err());
        assert!(transition_respond("responded").is_err());
        assert!(transition_respond("closed").is_err());
    }

    #[test]
    fn responded_to_closed() {
        assert_eq!(
            transition_close("responded").unwrap(),
            ComplaintStatus::Closed
        );
    }

    #[test]
    fn cannot_close_from_non_responded() {
        assert!(transition_close("intake").is_err());
        assert!(transition_close("triaged").is_err());
        assert!(transition_close("investigating").is_err());
        assert!(transition_close("closed").is_err());
        assert!(transition_close("cancelled").is_err());
    }

    #[test]
    fn cancel_from_any_non_terminal_succeeds() {
        for status in &["intake", "triaged", "investigating", "responded"] {
            assert_eq!(
                transition_cancel(status).unwrap(),
                ComplaintStatus::Cancelled
            );
        }
    }

    #[test]
    fn cancel_from_terminal_fails() {
        assert!(transition_cancel("closed").is_err());
        assert!(transition_cancel("cancelled").is_err());
    }
}
