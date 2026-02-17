//! Unit tests for the dunning state machine (pure logic — no DB).

use super::*;

// ─── Transition guard ────────────────────────────────────────────────────

#[test]
fn valid_transitions() {
    use DunningStateValue::*;
    let valid = [
        (Pending, Warned),
        (Pending, Escalated),
        (Pending, Resolved),
        (Pending, WrittenOff),
        (Warned, Escalated),
        (Warned, Resolved),
        (Warned, WrittenOff),
        (Escalated, Suspended),
        (Escalated, Resolved),
        (Escalated, WrittenOff),
        (Suspended, Resolved),
        (Suspended, WrittenOff),
    ];
    for (from, to) in valid {
        assert!(
            is_valid_transition(&from, &to),
            "Expected {} → {} to be valid",
            from,
            to
        );
    }
}

#[test]
fn invalid_transitions() {
    use DunningStateValue::*;
    let invalid = [
        (Resolved, Warned),
        (Resolved, Escalated),
        (Resolved, Suspended),
        (WrittenOff, Resolved),
        (WrittenOff, Pending),
        (Warned, Pending),       // no backwards
        (Escalated, Warned),     // no backwards
        (Suspended, Escalated),  // no backwards
    ];
    for (from, to) in invalid {
        assert!(
            !is_valid_transition(&from, &to),
            "Expected {} → {} to be invalid",
            from,
            to
        );
    }
}

#[test]
fn terminal_states() {
    assert!(DunningStateValue::Resolved.is_terminal());
    assert!(DunningStateValue::WrittenOff.is_terminal());
    assert!(!DunningStateValue::Pending.is_terminal());
    assert!(!DunningStateValue::Warned.is_terminal());
    assert!(!DunningStateValue::Escalated.is_terminal());
    assert!(!DunningStateValue::Suspended.is_terminal());
}

#[test]
fn state_from_str_roundtrip() {
    use DunningStateValue::*;
    let variants = [Pending, Warned, Escalated, Suspended, Resolved, WrittenOff];
    for v in variants {
        let s = v.as_str();
        let back = DunningStateValue::from_str(s).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn state_from_str_unknown_returns_none() {
    assert!(DunningStateValue::from_str("unknown_state").is_none());
    assert!(DunningStateValue::from_str("").is_none());
}

#[test]
fn error_display() {
    let e = DunningError::DunningNotFound {
        invoice_id: 42,
        app_id: "tenant-1".to_string(),
    };
    assert!(e.to_string().contains("42"));
    assert!(e.to_string().contains("tenant-1"));

    let e = DunningError::IllegalTransition {
        from_state: "resolved".to_string(),
        to_state: "warned".to_string(),
    };
    assert!(e.to_string().contains("resolved"));
    assert!(e.to_string().contains("warned"));

    let e = DunningError::TerminalState {
        state: "resolved".to_string(),
    };
    assert!(e.to_string().contains("terminal"));

    let e = DunningError::ConcurrentModification { invoice_id: 7 };
    assert!(e.to_string().contains("7"));
}
