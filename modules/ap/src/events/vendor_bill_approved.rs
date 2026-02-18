//! GL posting allocation line for the ap.vendor_bill_approved event.
//!
//! `ApprovedGlLine` is embedded in `VendorBillApprovedPayload` so the GL
//! consumer has all expense account routing information without re-reading
//! the AP database (replay-safe).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A GL posting allocation for one bill line, carried in the approved event.
///
/// The GL consumer reads these to build per-line expense debits:
/// - If `po_line_id` is Some → post to `AP_CLEARING` (inventory clearing)
/// - If `po_line_id` is None → post to `gl_account_code` (expense account)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovedGlLine {
    pub line_id: Uuid,
    /// GL account code for the expense debit (e.g. "6200", "6300").
    /// Ignored when `po_line_id` is Some — inventory clearing account is used instead.
    pub gl_account_code: String,
    /// Line total in minor currency units (same currency as the bill).
    pub amount_minor: i64,
    /// PO line reference. When present, line is PO-backed → post to AP_CLEARING.
    pub po_line_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_gl_line_serialises_round_trip() {
        let line = ApprovedGlLine {
            line_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            gl_account_code: "6200".to_string(),
            amount_minor: 10000,
            po_line_id: None,
        };
        let json = serde_json::to_string(&line).unwrap();
        let back: ApprovedGlLine = serde_json::from_str(&json).unwrap();
        assert_eq!(line, back);
    }

    #[test]
    fn approved_gl_line_po_backed_has_po_line_id() {
        let po_id = Uuid::new_v4();
        let line = ApprovedGlLine {
            line_id: Uuid::new_v4(),
            gl_account_code: "2100".to_string(),
            amount_minor: 5000,
            po_line_id: Some(po_id),
        };
        assert_eq!(line.po_line_id, Some(po_id));
    }
}
