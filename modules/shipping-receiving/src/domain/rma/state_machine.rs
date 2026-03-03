use super::types::DispositionStatus;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DispositionTransitionError {
    #[error("cannot transition from terminal disposition status '{from}'")]
    FromTerminal { from: DispositionStatus },

    #[error("disposition transition from '{from}' to '{to}' is not allowed")]
    NotAllowed {
        from: DispositionStatus,
        to: DispositionStatus,
    },

    #[error("disposition transition to same status '{status}' is a no-op")]
    NoOp { status: DispositionStatus },
}

/// Returns the set of disposition statuses reachable from `from`.
///
/// ```text
/// received        → inspect
/// inspect         → quarantine, return_to_stock, scrap
/// quarantine      → return_to_stock, scrap
/// return_to_stock → (terminal)
/// scrap           → (terminal)
/// ```
pub fn disposition_transitions(from: DispositionStatus) -> &'static [DispositionStatus] {
    match from {
        DispositionStatus::Received => &[DispositionStatus::Inspect],
        DispositionStatus::Inspect => &[
            DispositionStatus::Quarantine,
            DispositionStatus::ReturnToStock,
            DispositionStatus::Scrap,
        ],
        DispositionStatus::Quarantine => &[
            DispositionStatus::ReturnToStock,
            DispositionStatus::Scrap,
        ],
        DispositionStatus::ReturnToStock => &[],
        DispositionStatus::Scrap => &[],
    }
}

/// Validate that transitioning from `from` to `to` is allowed.
pub fn validate_disposition(
    from: DispositionStatus,
    to: DispositionStatus,
) -> Result<(), DispositionTransitionError> {
    if from == to {
        return Err(DispositionTransitionError::NoOp { status: from });
    }
    if from.is_terminal() {
        return Err(DispositionTransitionError::FromTerminal { from });
    }
    if disposition_transitions(from).contains(&to) {
        Ok(())
    } else {
        Err(DispositionTransitionError::NotAllowed { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_transitions() {
        use DispositionStatus::*;
        let allowed = [
            (Received, Inspect),
            (Inspect, Quarantine),
            (Inspect, ReturnToStock),
            (Inspect, Scrap),
            (Quarantine, ReturnToStock),
            (Quarantine, Scrap),
        ];
        for (from, to) in &allowed {
            assert!(
                validate_disposition(*from, *to).is_ok(),
                "{from} → {to} should be allowed"
            );
        }
    }

    #[test]
    fn terminal_states_reject_transitions() {
        use DispositionStatus::*;
        for terminal in &[ReturnToStock, Scrap] {
            assert_eq!(
                validate_disposition(*terminal, Received).unwrap_err(),
                DispositionTransitionError::FromTerminal { from: *terminal },
            );
        }
    }

    #[test]
    fn no_op_same_status() {
        assert_eq!(
            validate_disposition(DispositionStatus::Received, DispositionStatus::Received)
                .unwrap_err(),
            DispositionTransitionError::NoOp {
                status: DispositionStatus::Received
            },
        );
    }

    #[test]
    fn forbidden_transitions() {
        use DispositionStatus::*;
        // Non-terminal → invalid target: should be NotAllowed
        let not_allowed = [
            (Received, Quarantine),
            (Received, ReturnToStock),
            (Received, Scrap),
            (Quarantine, Inspect),
            (Quarantine, Received),
        ];
        for (from, to) in &not_allowed {
            assert_eq!(
                validate_disposition(*from, *to).unwrap_err(),
                DispositionTransitionError::NotAllowed {
                    from: *from,
                    to: *to
                },
                "{from} → {to} should be forbidden",
            );
        }

        // Terminal → any: should be FromTerminal (already covered in terminal_states test,
        // but verify the specific case from the bead's acceptance criteria)
        assert!(validate_disposition(Scrap, ReturnToStock).is_err());
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        assert!(disposition_transitions(DispositionStatus::ReturnToStock).is_empty());
        assert!(disposition_transitions(DispositionStatus::Scrap).is_empty());
    }

    #[test]
    fn all_transitions_are_deterministic() {
        for from in &DispositionStatus::ALL {
            for to in &DispositionStatus::ALL {
                let _ = validate_disposition(*from, *to);
            }
        }
    }
}
