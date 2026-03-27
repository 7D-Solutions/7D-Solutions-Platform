use super::types::{InboundStatus, LineQty, OutboundStatus};
use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuardError {
    #[error("arrived_at is required when transitioning to arrived")]
    MissingArrivedAt,

    #[error("shipped_at is required when transitioning to shipped")]
    MissingShippedAt,

    #[error("delivered_at is required when transitioning to delivered")]
    MissingDeliveredAt,

    #[error("closed_at is required when transitioning to closed")]
    MissingClosedAt,

    #[error("shipment has no lines — cannot close")]
    NoLines,

    #[error(
        "inbound close: line {line_id} qty_accepted ({accepted}) + \
         qty_rejected ({rejected}) != qty_received ({received})"
    )]
    InboundQtyMismatch {
        line_id: Uuid,
        accepted: i64,
        rejected: i64,
        received: i64,
    },

    #[error(
        "inbound close: line {line_id} qty_received ({received}) > \
         qty_expected ({expected})"
    )]
    InboundReceivedExceedsExpected {
        line_id: Uuid,
        received: i64,
        expected: i64,
    },

    #[error("outbound ship: line {line_id} qty_shipped ({shipped}) must be > 0")]
    OutboundQtyShippedZero { line_id: Uuid, shipped: i64 },

    #[error(
        "outbound ship: line {line_id} qty_shipped ({shipped}) > \
         qty_expected ({expected})"
    )]
    OutboundShippedExceedsExpected {
        line_id: Uuid,
        shipped: i64,
        expected: i64,
    },

    #[error("outbound ship: shipment has already been shipped (shipped_at is set)")]
    AlreadyShipped,
}

/// Context for inbound transition guards.
pub struct InboundGuardContext {
    pub arrived_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub lines: Vec<LineQty>,
    pub already_shipped_at: Option<DateTime<Utc>>,
}

/// Context for outbound transition guards.
pub struct OutboundGuardContext {
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub lines: Vec<LineQty>,
    pub already_shipped_at: Option<DateTime<Utc>>,
}

// ── Inbound guards ────────────────────────────────────────────

/// Validate inbound close invariant:
/// - Must have at least one line
/// - For each line: qty_accepted + qty_rejected == qty_received
/// - For each line: qty_received <= qty_expected
fn validate_inbound_close(ctx: &InboundGuardContext) -> Result<(), GuardError> {
    if ctx.closed_at.is_none() {
        return Err(GuardError::MissingClosedAt);
    }
    if ctx.lines.is_empty() {
        return Err(GuardError::NoLines);
    }
    for line in &ctx.lines {
        if line.qty_accepted + line.qty_rejected != line.qty_received {
            return Err(GuardError::InboundQtyMismatch {
                line_id: line.line_id,
                accepted: line.qty_accepted,
                rejected: line.qty_rejected,
                received: line.qty_received,
            });
        }
        if line.qty_received > line.qty_expected {
            return Err(GuardError::InboundReceivedExceedsExpected {
                line_id: line.line_id,
                received: line.qty_received,
                expected: line.qty_expected,
            });
        }
    }
    Ok(())
}

fn validate_inbound_arrived(ctx: &InboundGuardContext) -> Result<(), GuardError> {
    if ctx.arrived_at.is_none() {
        return Err(GuardError::MissingArrivedAt);
    }
    Ok(())
}

/// Run all inbound guards for the given target status.
pub fn run_inbound_guards(to: InboundStatus, ctx: &InboundGuardContext) -> Result<(), GuardError> {
    match to {
        InboundStatus::Arrived => validate_inbound_arrived(ctx),
        InboundStatus::Closed => validate_inbound_close(ctx),
        _ => Ok(()),
    }
}

// ── Outbound guards ──────────────────────────────────────────

/// Validate outbound ship invariant:
/// - Must have at least one line
/// - For each line: qty_shipped > 0
/// - For each line: qty_shipped <= qty_expected
/// - Ship once only: already_shipped_at must be None
fn validate_outbound_ship(ctx: &OutboundGuardContext) -> Result<(), GuardError> {
    if ctx.shipped_at.is_none() {
        return Err(GuardError::MissingShippedAt);
    }
    if ctx.already_shipped_at.is_some() {
        return Err(GuardError::AlreadyShipped);
    }
    if ctx.lines.is_empty() {
        return Err(GuardError::NoLines);
    }
    for line in &ctx.lines {
        if line.qty_shipped <= 0 {
            return Err(GuardError::OutboundQtyShippedZero {
                line_id: line.line_id,
                shipped: line.qty_shipped,
            });
        }
        if line.qty_shipped > line.qty_expected {
            return Err(GuardError::OutboundShippedExceedsExpected {
                line_id: line.line_id,
                shipped: line.qty_shipped,
                expected: line.qty_expected,
            });
        }
    }
    Ok(())
}

fn validate_outbound_delivered(ctx: &OutboundGuardContext) -> Result<(), GuardError> {
    if ctx.delivered_at.is_none() {
        return Err(GuardError::MissingDeliveredAt);
    }
    Ok(())
}

fn validate_outbound_close(ctx: &OutboundGuardContext) -> Result<(), GuardError> {
    if ctx.closed_at.is_none() {
        return Err(GuardError::MissingClosedAt);
    }
    Ok(())
}

/// Run all outbound guards for the given target status.
pub fn run_outbound_guards(
    to: OutboundStatus,
    ctx: &OutboundGuardContext,
) -> Result<(), GuardError> {
    match to {
        OutboundStatus::Shipped => validate_outbound_ship(ctx),
        OutboundStatus::Delivered => validate_outbound_delivered(ctx),
        OutboundStatus::Closed => validate_outbound_close(ctx),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_line(
        expected: i64,
        shipped: i64,
        received: i64,
        accepted: i64,
        rejected: i64,
    ) -> LineQty {
        LineQty {
            line_id: Uuid::new_v4(),
            sku: String::new(),
            qty_expected: expected,
            qty_shipped: shipped,
            qty_received: received,
            qty_accepted: accepted,
            qty_rejected: rejected,
            source_ref_type: None,
            source_ref_id: None,
        }
    }

    fn empty_inbound_ctx() -> InboundGuardContext {
        InboundGuardContext {
            arrived_at: None,
            closed_at: None,
            lines: vec![],
            already_shipped_at: None,
        }
    }

    fn empty_outbound_ctx() -> OutboundGuardContext {
        OutboundGuardContext {
            shipped_at: None,
            delivered_at: None,
            closed_at: None,
            lines: vec![],
            already_shipped_at: None,
        }
    }

    // ── Inbound arrived guards ────────────────────────────────

    #[test]
    fn inbound_arrived_requires_arrived_at() {
        let ctx = empty_inbound_ctx();
        assert_eq!(
            run_inbound_guards(InboundStatus::Arrived, &ctx),
            Err(GuardError::MissingArrivedAt)
        );
    }

    #[test]
    fn inbound_arrived_passes_with_timestamp() {
        let ctx = InboundGuardContext {
            arrived_at: Some(Utc::now()),
            ..empty_inbound_ctx()
        };
        assert!(run_inbound_guards(InboundStatus::Arrived, &ctx).is_ok());
    }

    // ── Inbound close guards ──────────────────────────────────

    #[test]
    fn inbound_close_requires_closed_at() {
        let ctx = InboundGuardContext {
            lines: vec![make_line(10, 0, 10, 8, 2)],
            ..empty_inbound_ctx()
        };
        assert_eq!(
            run_inbound_guards(InboundStatus::Closed, &ctx),
            Err(GuardError::MissingClosedAt)
        );
    }

    #[test]
    fn inbound_close_requires_lines() {
        let ctx = InboundGuardContext {
            closed_at: Some(Utc::now()),
            ..empty_inbound_ctx()
        };
        assert_eq!(
            run_inbound_guards(InboundStatus::Closed, &ctx),
            Err(GuardError::NoLines)
        );
    }

    #[test]
    fn inbound_close_qty_accepted_plus_rejected_must_equal_received() {
        let line = make_line(10, 0, 10, 7, 2); // 7 + 2 = 9 != 10
        let ctx = InboundGuardContext {
            closed_at: Some(Utc::now()),
            lines: vec![line.clone()],
            ..empty_inbound_ctx()
        };
        let err = run_inbound_guards(InboundStatus::Closed, &ctx).unwrap_err();
        match err {
            GuardError::InboundQtyMismatch {
                accepted,
                rejected,
                received,
                ..
            } => {
                assert_eq!(accepted, 7);
                assert_eq!(rejected, 2);
                assert_eq!(received, 10);
            }
            other => panic!("expected InboundQtyMismatch, got {:?}", other),
        }
    }

    #[test]
    fn inbound_close_qty_received_cannot_exceed_expected() {
        let line = make_line(10, 0, 11, 9, 2); // received 11 > expected 10
        let ctx = InboundGuardContext {
            closed_at: Some(Utc::now()),
            lines: vec![line],
            ..empty_inbound_ctx()
        };
        let err = run_inbound_guards(InboundStatus::Closed, &ctx).unwrap_err();
        match err {
            GuardError::InboundQtyMismatch { .. } => {} // mismatch fires first (9+2=11!=11 wait, 9+2=11==11)
            GuardError::InboundReceivedExceedsExpected {
                received, expected, ..
            } => {
                assert_eq!(received, 11);
                assert_eq!(expected, 10);
            }
            other => panic!("expected qty error, got {:?}", other),
        }
    }

    #[test]
    fn inbound_close_passes_with_valid_lines() {
        let ctx = InboundGuardContext {
            closed_at: Some(Utc::now()),
            lines: vec![
                make_line(10, 0, 10, 8, 2), // 8 + 2 = 10 ✓, 10 <= 10 ✓
                make_line(5, 0, 3, 3, 0),   // 3 + 0 = 3 ✓, 3 <= 5 ✓
            ],
            ..empty_inbound_ctx()
        };
        assert!(run_inbound_guards(InboundStatus::Closed, &ctx).is_ok());
    }

    #[test]
    fn inbound_close_passes_with_zero_quantities() {
        let ctx = InboundGuardContext {
            closed_at: Some(Utc::now()),
            lines: vec![make_line(10, 0, 0, 0, 0)], // 0 + 0 = 0 ✓, 0 <= 10 ✓
            ..empty_inbound_ctx()
        };
        assert!(run_inbound_guards(InboundStatus::Closed, &ctx).is_ok());
    }

    // ── Inbound non-guarded statuses pass ─────────────────────

    #[test]
    fn inbound_non_guarded_statuses_pass() {
        let ctx = empty_inbound_ctx();
        assert!(run_inbound_guards(InboundStatus::Draft, &ctx).is_ok());
        assert!(run_inbound_guards(InboundStatus::Confirmed, &ctx).is_ok());
        assert!(run_inbound_guards(InboundStatus::InTransit, &ctx).is_ok());
        assert!(run_inbound_guards(InboundStatus::Receiving, &ctx).is_ok());
        assert!(run_inbound_guards(InboundStatus::Cancelled, &ctx).is_ok());
    }

    // ── Outbound ship guards ──────────────────────────────────

    #[test]
    fn outbound_ship_requires_shipped_at() {
        let ctx = OutboundGuardContext {
            lines: vec![make_line(10, 5, 0, 0, 0)],
            ..empty_outbound_ctx()
        };
        assert_eq!(
            run_outbound_guards(OutboundStatus::Shipped, &ctx),
            Err(GuardError::MissingShippedAt)
        );
    }

    #[test]
    fn outbound_ship_prevents_double_ship() {
        let ctx = OutboundGuardContext {
            shipped_at: Some(Utc::now()),
            already_shipped_at: Some(Utc::now()),
            lines: vec![make_line(10, 5, 0, 0, 0)],
            ..empty_outbound_ctx()
        };
        assert_eq!(
            run_outbound_guards(OutboundStatus::Shipped, &ctx),
            Err(GuardError::AlreadyShipped)
        );
    }

    #[test]
    fn outbound_ship_requires_lines() {
        let ctx = OutboundGuardContext {
            shipped_at: Some(Utc::now()),
            ..empty_outbound_ctx()
        };
        assert_eq!(
            run_outbound_guards(OutboundStatus::Shipped, &ctx),
            Err(GuardError::NoLines)
        );
    }

    #[test]
    fn outbound_ship_qty_shipped_must_be_positive() {
        let line = make_line(10, 0, 0, 0, 0); // qty_shipped = 0
        let ctx = OutboundGuardContext {
            shipped_at: Some(Utc::now()),
            lines: vec![line.clone()],
            ..empty_outbound_ctx()
        };
        let err = run_outbound_guards(OutboundStatus::Shipped, &ctx).unwrap_err();
        match err {
            GuardError::OutboundQtyShippedZero { shipped, .. } => {
                assert_eq!(shipped, 0);
            }
            other => panic!("expected OutboundQtyShippedZero, got {:?}", other),
        }
    }

    #[test]
    fn outbound_ship_qty_shipped_cannot_exceed_expected() {
        let line = make_line(10, 11, 0, 0, 0); // shipped 11 > expected 10
        let ctx = OutboundGuardContext {
            shipped_at: Some(Utc::now()),
            lines: vec![line],
            ..empty_outbound_ctx()
        };
        let err = run_outbound_guards(OutboundStatus::Shipped, &ctx).unwrap_err();
        match err {
            GuardError::OutboundShippedExceedsExpected {
                shipped, expected, ..
            } => {
                assert_eq!(shipped, 11);
                assert_eq!(expected, 10);
            }
            other => panic!("expected OutboundShippedExceedsExpected, got {:?}", other),
        }
    }

    #[test]
    fn outbound_ship_passes_with_valid_lines() {
        let ctx = OutboundGuardContext {
            shipped_at: Some(Utc::now()),
            lines: vec![
                make_line(10, 10, 0, 0, 0), // shipped == expected ✓
                make_line(5, 3, 0, 0, 0),   // shipped < expected ✓
            ],
            ..empty_outbound_ctx()
        };
        assert!(run_outbound_guards(OutboundStatus::Shipped, &ctx).is_ok());
    }

    // ── Outbound delivered guards ─────────────────────────────

    #[test]
    fn outbound_delivered_requires_delivered_at() {
        let ctx = empty_outbound_ctx();
        assert_eq!(
            run_outbound_guards(OutboundStatus::Delivered, &ctx),
            Err(GuardError::MissingDeliveredAt)
        );
    }

    #[test]
    fn outbound_delivered_passes_with_timestamp() {
        let ctx = OutboundGuardContext {
            delivered_at: Some(Utc::now()),
            ..empty_outbound_ctx()
        };
        assert!(run_outbound_guards(OutboundStatus::Delivered, &ctx).is_ok());
    }

    // ── Outbound close guards ─────────────────────────────────

    #[test]
    fn outbound_close_requires_closed_at() {
        let ctx = empty_outbound_ctx();
        assert_eq!(
            run_outbound_guards(OutboundStatus::Closed, &ctx),
            Err(GuardError::MissingClosedAt)
        );
    }

    #[test]
    fn outbound_close_passes_with_timestamp() {
        let ctx = OutboundGuardContext {
            closed_at: Some(Utc::now()),
            ..empty_outbound_ctx()
        };
        assert!(run_outbound_guards(OutboundStatus::Closed, &ctx).is_ok());
    }

    // ── Outbound non-guarded statuses pass ────────────────────

    #[test]
    fn outbound_non_guarded_statuses_pass() {
        let ctx = empty_outbound_ctx();
        assert!(run_outbound_guards(OutboundStatus::Draft, &ctx).is_ok());
        assert!(run_outbound_guards(OutboundStatus::Confirmed, &ctx).is_ok());
        assert!(run_outbound_guards(OutboundStatus::Picking, &ctx).is_ok());
        assert!(run_outbound_guards(OutboundStatus::Packed, &ctx).is_ok());
        assert!(run_outbound_guards(OutboundStatus::Cancelled, &ctx).is_ok());
    }
}
