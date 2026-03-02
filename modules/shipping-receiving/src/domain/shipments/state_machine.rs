use super::types::{InboundStatus, OutboundStatus};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("cannot transition from terminal inbound status '{from}'")]
    InboundFromTerminal { from: InboundStatus },

    #[error("inbound transition from '{from}' to '{to}' is not allowed")]
    InboundNotAllowed {
        from: InboundStatus,
        to: InboundStatus,
    },

    #[error("inbound transition to same status '{status}' is a no-op")]
    InboundNoOp { status: InboundStatus },

    #[error("cannot transition from terminal outbound status '{from}'")]
    OutboundFromTerminal { from: OutboundStatus },

    #[error("outbound transition from '{from}' to '{to}' is not allowed")]
    OutboundNotAllowed {
        from: OutboundStatus,
        to: OutboundStatus,
    },

    #[error("outbound transition to same status '{status}' is a no-op")]
    OutboundNoOp { status: OutboundStatus },
}

// ── Inbound state machine ─────────────────────────────────────

/// Returns the set of inbound statuses reachable from `from`.
///
/// ```text
/// draft      → confirmed, cancelled
/// confirmed  → in_transit, cancelled
/// in_transit → arrived, cancelled
/// arrived    → receiving, cancelled
/// receiving  → closed, cancelled
/// closed     → (terminal)
/// cancelled  → (terminal)
/// ```
pub fn inbound_transitions(from: InboundStatus) -> &'static [InboundStatus] {
    match from {
        InboundStatus::Draft => &[InboundStatus::Confirmed, InboundStatus::Cancelled],
        InboundStatus::Confirmed => &[InboundStatus::InTransit, InboundStatus::Cancelled],
        InboundStatus::InTransit => &[InboundStatus::Arrived, InboundStatus::Cancelled],
        InboundStatus::Arrived => &[InboundStatus::Receiving, InboundStatus::Cancelled],
        InboundStatus::Receiving => &[InboundStatus::Closed, InboundStatus::Cancelled],
        InboundStatus::Closed => &[],
        InboundStatus::Cancelled => &[],
    }
}

/// Validate that transitioning from `from` to `to` is allowed for inbound shipments.
pub fn validate_inbound(from: InboundStatus, to: InboundStatus) -> Result<(), TransitionError> {
    if from == to {
        return Err(TransitionError::InboundNoOp { status: from });
    }
    if from.is_terminal() {
        return Err(TransitionError::InboundFromTerminal { from });
    }
    if inbound_transitions(from).contains(&to) {
        Ok(())
    } else {
        Err(TransitionError::InboundNotAllowed { from, to })
    }
}

// ── Outbound state machine ────────────────────────────────────

/// Returns the set of outbound statuses reachable from `from`.
///
/// ```text
/// draft     → confirmed, cancelled
/// confirmed → picking, cancelled
/// picking   → packed, cancelled
/// packed    → shipped, cancelled
/// shipped   → delivered, cancelled
/// delivered → closed, cancelled
/// closed    → (terminal)
/// cancelled → (terminal)
/// ```
pub fn outbound_transitions(from: OutboundStatus) -> &'static [OutboundStatus] {
    match from {
        OutboundStatus::Draft => &[OutboundStatus::Confirmed, OutboundStatus::Cancelled],
        OutboundStatus::Confirmed => &[OutboundStatus::Picking, OutboundStatus::Cancelled],
        OutboundStatus::Picking => &[OutboundStatus::Packed, OutboundStatus::Cancelled],
        OutboundStatus::Packed => &[OutboundStatus::Shipped, OutboundStatus::Cancelled],
        OutboundStatus::Shipped => &[OutboundStatus::Delivered, OutboundStatus::Cancelled],
        OutboundStatus::Delivered => &[OutboundStatus::Closed, OutboundStatus::Cancelled],
        OutboundStatus::Closed => &[],
        OutboundStatus::Cancelled => &[],
    }
}

/// Validate that transitioning from `from` to `to` is allowed for outbound shipments.
pub fn validate_outbound(from: OutboundStatus, to: OutboundStatus) -> Result<(), TransitionError> {
    if from == to {
        return Err(TransitionError::OutboundNoOp { status: from });
    }
    if from.is_terminal() {
        return Err(TransitionError::OutboundFromTerminal { from });
    }
    if outbound_transitions(from).contains(&to) {
        Ok(())
    } else {
        Err(TransitionError::OutboundNotAllowed { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Inbound: allowed transitions (table-driven) ───────────

    #[test]
    fn inbound_allowed_transitions() {
        use InboundStatus::*;
        let allowed = [
            (Draft, Confirmed),
            (Draft, Cancelled),
            (Confirmed, InTransit),
            (Confirmed, Cancelled),
            (InTransit, Arrived),
            (InTransit, Cancelled),
            (Arrived, Receiving),
            (Arrived, Cancelled),
            (Receiving, Closed),
            (Receiving, Cancelled),
        ];
        for (from, to) in &allowed {
            assert!(
                validate_inbound(*from, *to).is_ok(),
                "{from} → {to} should be allowed"
            );
        }
    }

    // ── Inbound: forbidden transitions ────────────────────────

    #[test]
    fn inbound_closed_is_terminal() {
        assert_eq!(
            validate_inbound(InboundStatus::Closed, InboundStatus::Draft).unwrap_err(),
            TransitionError::InboundFromTerminal {
                from: InboundStatus::Closed
            },
        );
    }

    #[test]
    fn inbound_cancelled_is_terminal() {
        assert_eq!(
            validate_inbound(InboundStatus::Cancelled, InboundStatus::Draft).unwrap_err(),
            TransitionError::InboundFromTerminal {
                from: InboundStatus::Cancelled
            },
        );
    }

    #[test]
    fn inbound_no_op_same_status() {
        assert_eq!(
            validate_inbound(InboundStatus::Draft, InboundStatus::Draft).unwrap_err(),
            TransitionError::InboundNoOp {
                status: InboundStatus::Draft
            },
        );
    }

    #[test]
    fn inbound_forbidden_skips() {
        use InboundStatus::*;
        let forbidden = [(Draft, Arrived), (Draft, Closed), (Arrived, Closed)];
        for (from, to) in &forbidden {
            assert_eq!(
                validate_inbound(*from, *to).unwrap_err(),
                TransitionError::InboundNotAllowed {
                    from: *from,
                    to: *to
                },
                "{from} → {to} should be forbidden",
            );
        }
    }

    // ── Inbound: exhaustive coverage ──────────────────────────

    #[test]
    fn inbound_all_transitions_are_deterministic() {
        for from in &InboundStatus::ALL {
            for to in &InboundStatus::ALL {
                let _ = validate_inbound(*from, *to);
            }
        }
    }

    #[test]
    fn inbound_terminal_states_have_no_outgoing_edges() {
        assert!(inbound_transitions(InboundStatus::Closed).is_empty());
        assert!(inbound_transitions(InboundStatus::Cancelled).is_empty());
    }

    #[test]
    fn inbound_every_non_terminal_can_be_cancelled() {
        use InboundStatus::*;
        for status in &[Draft, Confirmed, InTransit, Arrived, Receiving] {
            assert!(
                validate_inbound(*status, Cancelled).is_ok(),
                "{status} should be cancellable",
            );
        }
    }

    // ── Outbound: allowed transitions (table-driven) ──────────

    #[test]
    fn outbound_allowed_transitions() {
        use OutboundStatus::*;
        let allowed = [
            (Draft, Confirmed),
            (Draft, Cancelled),
            (Confirmed, Picking),
            (Confirmed, Cancelled),
            (Picking, Packed),
            (Picking, Cancelled),
            (Packed, Shipped),
            (Packed, Cancelled),
            (Shipped, Delivered),
            (Shipped, Cancelled),
            (Delivered, Closed),
            (Delivered, Cancelled),
        ];
        for (from, to) in &allowed {
            assert!(
                validate_outbound(*from, *to).is_ok(),
                "{from} → {to} should be allowed"
            );
        }
    }

    // ── Outbound: forbidden transitions ───────────────────────

    #[test]
    fn outbound_closed_is_terminal() {
        assert_eq!(
            validate_outbound(OutboundStatus::Closed, OutboundStatus::Draft).unwrap_err(),
            TransitionError::OutboundFromTerminal {
                from: OutboundStatus::Closed
            },
        );
    }

    #[test]
    fn outbound_cancelled_is_terminal() {
        assert_eq!(
            validate_outbound(OutboundStatus::Cancelled, OutboundStatus::Draft).unwrap_err(),
            TransitionError::OutboundFromTerminal {
                from: OutboundStatus::Cancelled
            },
        );
    }

    #[test]
    fn outbound_no_op_same_status() {
        assert_eq!(
            validate_outbound(OutboundStatus::Draft, OutboundStatus::Draft).unwrap_err(),
            TransitionError::OutboundNoOp {
                status: OutboundStatus::Draft
            },
        );
    }

    #[test]
    fn outbound_forbidden_skips() {
        use OutboundStatus::*;
        let forbidden = [
            (Draft, Shipped),
            (Draft, Closed),
            (Packed, Delivered),
            (Shipped, Closed),
        ];
        for (from, to) in &forbidden {
            assert_eq!(
                validate_outbound(*from, *to).unwrap_err(),
                TransitionError::OutboundNotAllowed {
                    from: *from,
                    to: *to
                },
                "{from} → {to} should be forbidden",
            );
        }
    }

    // ── Outbound: exhaustive coverage ─────────────────────────

    #[test]
    fn outbound_all_transitions_are_deterministic() {
        for from in &OutboundStatus::ALL {
            for to in &OutboundStatus::ALL {
                let _ = validate_outbound(*from, *to);
            }
        }
    }

    #[test]
    fn outbound_terminal_states_have_no_outgoing_edges() {
        assert!(outbound_transitions(OutboundStatus::Closed).is_empty());
        assert!(outbound_transitions(OutboundStatus::Cancelled).is_empty());
    }

    #[test]
    fn outbound_every_non_terminal_can_be_cancelled() {
        use OutboundStatus::*;
        for status in &[Draft, Confirmed, Picking, Packed, Shipped, Delivered] {
            assert!(
                validate_outbound(*status, Cancelled).is_ok(),
                "{status} should be cancellable",
            );
        }
    }
}
