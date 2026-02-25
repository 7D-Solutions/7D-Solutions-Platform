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
pub fn validate_outbound(
    from: OutboundStatus,
    to: OutboundStatus,
) -> Result<(), TransitionError> {
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

    // ── Inbound: allowed transitions ──────────────────────────

    #[test]
    fn inbound_draft_to_confirmed() {
        assert!(validate_inbound(InboundStatus::Draft, InboundStatus::Confirmed).is_ok());
    }

    #[test]
    fn inbound_draft_to_cancelled() {
        assert!(validate_inbound(InboundStatus::Draft, InboundStatus::Cancelled).is_ok());
    }

    #[test]
    fn inbound_confirmed_to_in_transit() {
        assert!(
            validate_inbound(InboundStatus::Confirmed, InboundStatus::InTransit).is_ok()
        );
    }

    #[test]
    fn inbound_confirmed_to_cancelled() {
        assert!(
            validate_inbound(InboundStatus::Confirmed, InboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn inbound_in_transit_to_arrived() {
        assert!(
            validate_inbound(InboundStatus::InTransit, InboundStatus::Arrived).is_ok()
        );
    }

    #[test]
    fn inbound_in_transit_to_cancelled() {
        assert!(
            validate_inbound(InboundStatus::InTransit, InboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn inbound_arrived_to_receiving() {
        assert!(
            validate_inbound(InboundStatus::Arrived, InboundStatus::Receiving).is_ok()
        );
    }

    #[test]
    fn inbound_arrived_to_cancelled() {
        assert!(
            validate_inbound(InboundStatus::Arrived, InboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn inbound_receiving_to_closed() {
        assert!(
            validate_inbound(InboundStatus::Receiving, InboundStatus::Closed).is_ok()
        );
    }

    #[test]
    fn inbound_receiving_to_cancelled() {
        assert!(
            validate_inbound(InboundStatus::Receiving, InboundStatus::Cancelled).is_ok()
        );
    }

    // ── Inbound: forbidden transitions ────────────────────────

    #[test]
    fn inbound_closed_is_terminal() {
        let err = validate_inbound(InboundStatus::Closed, InboundStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::InboundFromTerminal {
                from: InboundStatus::Closed
            }
        );
    }

    #[test]
    fn inbound_cancelled_is_terminal() {
        let err =
            validate_inbound(InboundStatus::Cancelled, InboundStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::InboundFromTerminal {
                from: InboundStatus::Cancelled
            }
        );
    }

    #[test]
    fn inbound_no_op_same_status() {
        let err = validate_inbound(InboundStatus::Draft, InboundStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::InboundNoOp {
                status: InboundStatus::Draft
            }
        );
    }

    #[test]
    fn inbound_draft_cannot_skip_to_arrived() {
        let err =
            validate_inbound(InboundStatus::Draft, InboundStatus::Arrived).unwrap_err();
        assert_eq!(
            err,
            TransitionError::InboundNotAllowed {
                from: InboundStatus::Draft,
                to: InboundStatus::Arrived
            }
        );
    }

    #[test]
    fn inbound_draft_cannot_go_to_closed() {
        let err = validate_inbound(InboundStatus::Draft, InboundStatus::Closed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::InboundNotAllowed {
                from: InboundStatus::Draft,
                to: InboundStatus::Closed
            }
        );
    }

    #[test]
    fn inbound_arrived_cannot_go_to_closed() {
        let err =
            validate_inbound(InboundStatus::Arrived, InboundStatus::Closed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::InboundNotAllowed {
                from: InboundStatus::Arrived,
                to: InboundStatus::Closed
            }
        );
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
        let non_terminal = [
            InboundStatus::Draft,
            InboundStatus::Confirmed,
            InboundStatus::InTransit,
            InboundStatus::Arrived,
            InboundStatus::Receiving,
        ];
        for status in &non_terminal {
            assert!(
                validate_inbound(*status, InboundStatus::Cancelled).is_ok(),
                "{} should be cancellable",
                status
            );
        }
    }

    // ── Outbound: allowed transitions ─────────────────────────

    #[test]
    fn outbound_draft_to_confirmed() {
        assert!(
            validate_outbound(OutboundStatus::Draft, OutboundStatus::Confirmed).is_ok()
        );
    }

    #[test]
    fn outbound_draft_to_cancelled() {
        assert!(
            validate_outbound(OutboundStatus::Draft, OutboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn outbound_confirmed_to_picking() {
        assert!(
            validate_outbound(OutboundStatus::Confirmed, OutboundStatus::Picking).is_ok()
        );
    }

    #[test]
    fn outbound_confirmed_to_cancelled() {
        assert!(
            validate_outbound(OutboundStatus::Confirmed, OutboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn outbound_picking_to_packed() {
        assert!(
            validate_outbound(OutboundStatus::Picking, OutboundStatus::Packed).is_ok()
        );
    }

    #[test]
    fn outbound_picking_to_cancelled() {
        assert!(
            validate_outbound(OutboundStatus::Picking, OutboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn outbound_packed_to_shipped() {
        assert!(
            validate_outbound(OutboundStatus::Packed, OutboundStatus::Shipped).is_ok()
        );
    }

    #[test]
    fn outbound_packed_to_cancelled() {
        assert!(
            validate_outbound(OutboundStatus::Packed, OutboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn outbound_shipped_to_delivered() {
        assert!(
            validate_outbound(OutboundStatus::Shipped, OutboundStatus::Delivered).is_ok()
        );
    }

    #[test]
    fn outbound_shipped_to_cancelled() {
        assert!(
            validate_outbound(OutboundStatus::Shipped, OutboundStatus::Cancelled).is_ok()
        );
    }

    #[test]
    fn outbound_delivered_to_closed() {
        assert!(
            validate_outbound(OutboundStatus::Delivered, OutboundStatus::Closed).is_ok()
        );
    }

    #[test]
    fn outbound_delivered_to_cancelled() {
        assert!(
            validate_outbound(OutboundStatus::Delivered, OutboundStatus::Cancelled).is_ok()
        );
    }

    // ── Outbound: forbidden transitions ───────────────────────

    #[test]
    fn outbound_closed_is_terminal() {
        let err =
            validate_outbound(OutboundStatus::Closed, OutboundStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundFromTerminal {
                from: OutboundStatus::Closed
            }
        );
    }

    #[test]
    fn outbound_cancelled_is_terminal() {
        let err =
            validate_outbound(OutboundStatus::Cancelled, OutboundStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundFromTerminal {
                from: OutboundStatus::Cancelled
            }
        );
    }

    #[test]
    fn outbound_no_op_same_status() {
        let err =
            validate_outbound(OutboundStatus::Draft, OutboundStatus::Draft).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundNoOp {
                status: OutboundStatus::Draft
            }
        );
    }

    #[test]
    fn outbound_draft_cannot_skip_to_shipped() {
        let err =
            validate_outbound(OutboundStatus::Draft, OutboundStatus::Shipped).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundNotAllowed {
                from: OutboundStatus::Draft,
                to: OutboundStatus::Shipped
            }
        );
    }

    #[test]
    fn outbound_draft_cannot_go_to_closed() {
        let err =
            validate_outbound(OutboundStatus::Draft, OutboundStatus::Closed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundNotAllowed {
                from: OutboundStatus::Draft,
                to: OutboundStatus::Closed
            }
        );
    }

    #[test]
    fn outbound_packed_cannot_go_to_delivered() {
        let err =
            validate_outbound(OutboundStatus::Packed, OutboundStatus::Delivered).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundNotAllowed {
                from: OutboundStatus::Packed,
                to: OutboundStatus::Delivered
            }
        );
    }

    #[test]
    fn outbound_shipped_cannot_go_to_closed() {
        let err =
            validate_outbound(OutboundStatus::Shipped, OutboundStatus::Closed).unwrap_err();
        assert_eq!(
            err,
            TransitionError::OutboundNotAllowed {
                from: OutboundStatus::Shipped,
                to: OutboundStatus::Closed
            }
        );
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
        let non_terminal = [
            OutboundStatus::Draft,
            OutboundStatus::Confirmed,
            OutboundStatus::Picking,
            OutboundStatus::Packed,
            OutboundStatus::Shipped,
            OutboundStatus::Delivered,
        ];
        for status in &non_terminal {
            assert!(
                validate_outbound(*status, OutboundStatus::Cancelled).is_ok(),
                "{} should be cancellable",
                status
            );
        }
    }
}
