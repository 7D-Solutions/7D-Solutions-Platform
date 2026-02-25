//! Credit card reconciliation strategy.
//!
//! Matches CC statement lines to CC transactions using:
//! - Exact amount match (mandatory)
//! - Auth→settle date window tolerance
//! - Merchant name matching
//! - Reference fallback for fee lines (annual fees, interest, etc.)
//!
//! Refunds/credits are naturally separated by amount sign: purchases are
//! negative (money out), credits/refunds are positive (money back). Exact
//! amount matching ensures a refund line only matches a refund transaction.

use rust_decimal::Decimal;

use super::MatchStrategy;
use crate::domain::recon::models::UnmatchedTxn;

/// CC-specific matching with auth/settle date windows and merchant comparison.
pub struct CreditCardStrategy;

impl MatchStrategy for CreditCardStrategy {
    fn score(&self, sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Option<Decimal> {
        // Mandatory: amount + currency must match exactly
        if sl.amount_minor != pt.amount_minor {
            return None;
        }
        if sl.currency != pt.currency {
            return None;
        }

        let mut confidence = Decimal::new(5000, 4);

        // Date scoring using auth/settle window (up to +0.3)
        confidence += date_score(sl, pt);

        // Merchant matching (up to +0.2)
        let merchant = merchant_score(sl, pt);
        confidence += merchant;

        // Reference fallback (up to +0.1) only when no merchant match
        if merchant == Decimal::ZERO {
            confidence += reference_score(sl, pt);
        }

        Some(confidence)
    }
}

/// Score date proximity for CC transactions.
///
/// When the payment transaction has auth_date/settle_date, the statement line's
/// date should fall within `[auth_date - 1, settle_date + 3]` to account for
/// processing delays. Score is higher closer to settle_date.
///
/// Falls back to transaction_date comparison with wider tolerance (5 days)
/// when auth/settle dates are not available.
fn date_score(sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Decimal {
    // If payment txn has auth/settle window, use that for matching
    if let (Some(auth), Some(settle)) = (pt.auth_date, pt.settle_date) {
        let stmt_date = sl.transaction_date;
        let window_start = auth - chrono::Duration::days(1);
        let window_end = settle + chrono::Duration::days(3);

        if stmt_date < window_start || stmt_date > window_end {
            return Decimal::ZERO;
        }

        // Score based on proximity to settle date
        let days_from_settle = (stmt_date - settle).num_days().unsigned_abs();
        return match days_from_settle {
            0 => Decimal::new(3000, 4),
            1 => Decimal::new(2500, 4),
            2 => Decimal::new(2000, 4),
            3 => Decimal::new(1500, 4),
            _ => Decimal::new(1000, 4),
        };
    }

    // Fallback: wider date tolerance for CC (5 days vs 3 for bank)
    let day_diff = (sl.transaction_date - pt.transaction_date)
        .num_days()
        .unsigned_abs();
    match day_diff {
        0 => Decimal::new(3000, 4),
        1 => Decimal::new(2000, 4),
        2 => Decimal::new(1500, 4),
        3 => Decimal::new(1000, 4),
        4..=5 => Decimal::new(500, 4),
        _ => Decimal::ZERO,
    }
}

/// Compare merchant names — exact match → +0.2, substring containment → +0.1, else 0.
fn merchant_score(sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Decimal {
    match (&sl.merchant_name, &pt.merchant_name) {
        (Some(a), Some(b)) => {
            let a = a.trim().to_lowercase();
            let b = b.trim().to_lowercase();
            if a == b && !a.is_empty() {
                Decimal::new(2000, 4)
            } else if (!a.is_empty() && b.contains(&a)) || (!b.is_empty() && a.contains(&b)) {
                Decimal::new(1000, 4)
            } else {
                Decimal::ZERO
            }
        }
        _ => Decimal::ZERO,
    }
}

/// Fallback reference similarity for fee lines with no merchant descriptor.
fn reference_score(sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Decimal {
    match (sl.reference.as_deref(), pt.reference.as_deref()) {
        (Some(a), Some(b)) => {
            let a = a.trim().to_lowercase();
            let b = b.trim().to_lowercase();
            if a == b && !a.is_empty() {
                Decimal::new(1000, 4)
            } else if (!a.is_empty() && b.contains(&a)) || (!b.is_empty() && a.contains(&b)) {
                Decimal::new(500, 4)
            } else {
                Decimal::ZERO
            }
        }
        _ => Decimal::ZERO,
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::str::FromStr;
    use uuid::Uuid;

    fn make_cc_txn(
        amount: i64,
        date: NaiveDate,
        auth_date: Option<NaiveDate>,
        settle_date: Option<NaiveDate>,
        merchant: Option<&str>,
        reference: Option<&str>,
        has_statement: bool,
    ) -> UnmatchedTxn {
        UnmatchedTxn {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            transaction_date: date,
            amount_minor: amount,
            currency: "USD".to_string(),
            description: Some("test".to_string()),
            reference: reference.map(String::from),
            statement_id: if has_statement {
                Some(Uuid::new_v4())
            } else {
                None
            },
            auth_date,
            settle_date,
            merchant_name: merchant.map(String::from),
        }
    }

    #[test]
    fn cc_exact_match_max_confidence() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let strategy = CreditCardStrategy;
        let sl = make_cc_txn(-4500, d, Some(d), Some(d), Some("AMAZON"), None, true);
        let pt = make_cc_txn(-4500, d, Some(d), Some(d), Some("AMAZON"), None, false);
        let score = strategy.score(&sl, &pt).unwrap();
        // 0.5 base + 0.3 date (settle exact) + 0.2 merchant (exact) = 1.0
        assert_eq!(score, Decimal::from_str("1.0000").unwrap());
    }

    #[test]
    fn cc_amount_mismatch_no_match() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let strategy = CreditCardStrategy;
        let sl = make_cc_txn(-4500, d, None, None, Some("AMAZON"), None, true);
        let pt = make_cc_txn(-4501, d, None, None, Some("AMAZON"), None, false);
        assert!(strategy.score(&sl, &pt).is_none());
    }

    #[test]
    fn cc_currency_mismatch_no_match() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let strategy = CreditCardStrategy;
        let sl = make_cc_txn(-4500, d, None, None, None, None, true);
        let mut pt = make_cc_txn(-4500, d, None, None, None, None, false);
        pt.currency = "EUR".to_string();
        assert!(strategy.score(&sl, &pt).is_none());
    }

    #[test]
    fn cc_auth_settle_window_within() {
        let auth = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
        let settle = NaiveDate::from_ymd_opt(2024, 1, 13).unwrap();
        let stmt_date = NaiveDate::from_ymd_opt(2024, 1, 13).unwrap(); // = settle
        let strategy = CreditCardStrategy;

        let sl = make_cc_txn(-2000, stmt_date, None, None, Some("STORE"), None, true);
        let pt = make_cc_txn(-2000, auth, Some(auth), Some(settle), Some("STORE"), None, false);
        let score = strategy.score(&sl, &pt).unwrap();
        // 0.5 + 0.3 (settle exact) + 0.2 (merchant exact) = 1.0
        assert_eq!(score, Decimal::from_str("1.0000").unwrap());
    }

    #[test]
    fn cc_auth_settle_window_outside() {
        let auth = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
        let settle = NaiveDate::from_ymd_opt(2024, 1, 13).unwrap();
        // settle + 3 = Jan 16, so Jan 17 is outside
        let stmt_date = NaiveDate::from_ymd_opt(2024, 1, 17).unwrap();
        let strategy = CreditCardStrategy;

        let sl = make_cc_txn(-2000, stmt_date, None, None, Some("STORE"), None, true);
        let pt = make_cc_txn(-2000, auth, Some(auth), Some(settle), Some("STORE"), None, false);
        let score = strategy.score(&sl, &pt).unwrap();
        // Date score = 0 (outside window), merchant still matches
        // 0.5 + 0.0 + 0.2 = 0.7
        assert_eq!(score, Decimal::from_str("0.7000").unwrap());
    }

    #[test]
    fn cc_refund_matches_refund_only() {
        let d = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
        let strategy = CreditCardStrategy;

        // Refund line (+500) on statement
        let refund_sl = make_cc_txn(500, d, None, None, Some("AMAZON"), None, true);
        // Original purchase (-500) — should NOT match (sign differs)
        let purchase_pt = make_cc_txn(-500, d, None, None, Some("AMAZON"), None, false);
        assert!(strategy.score(&refund_sl, &purchase_pt).is_none());

        // Refund txn (+500) — should match
        let refund_pt = make_cc_txn(500, d, None, None, Some("AMAZON"), None, false);
        assert!(strategy.score(&refund_sl, &refund_pt).is_some());
    }

    #[test]
    fn cc_fee_line_uses_reference_fallback() {
        let d = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let strategy = CreditCardStrategy;

        // Fee line — no merchant, has reference
        let sl = make_cc_txn(-9900, d, None, None, None, Some("ANNUAL FEE"), true);
        let pt = make_cc_txn(-9900, d, None, None, None, Some("ANNUAL FEE"), false);
        let score = strategy.score(&sl, &pt).unwrap();
        // 0.5 + 0.3 (date exact) + 0.0 (no merchant) + 0.1 (reference exact) = 0.9
        assert_eq!(score, Decimal::from_str("0.9000").unwrap());
    }

    #[test]
    fn cc_merchant_partial_match() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let strategy = CreditCardStrategy;
        let sl = make_cc_txn(-3000, d, None, None, Some("AMAZON.COM"), None, true);
        let pt = make_cc_txn(-3000, d, None, None, Some("AMAZON"), None, false);
        let score = strategy.score(&sl, &pt).unwrap();
        // 0.5 + 0.3 (date exact) + 0.1 (merchant partial) = 0.9
        assert_eq!(score, Decimal::from_str("0.9000").unwrap());
    }

    #[test]
    fn cc_wider_date_tolerance() {
        let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(); // 5 days apart
        let strategy = CreditCardStrategy;

        let sl = make_cc_txn(-1000, d1, None, None, None, None, true);
        let pt = make_cc_txn(-1000, d2, None, None, None, None, false);
        let score = strategy.score(&sl, &pt).unwrap();
        // 0.5 + 0.05 (5 days) = 0.55
        assert_eq!(score, Decimal::from_str("0.5500").unwrap());
    }

    #[test]
    fn cc_no_date_bonus_beyond_5_days() {
        let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2024, 1, 22).unwrap(); // 7 days
        let strategy = CreditCardStrategy;

        let sl = make_cc_txn(-1000, d1, None, None, None, None, true);
        let pt = make_cc_txn(-1000, d2, None, None, None, None, false);
        let score = strategy.score(&sl, &pt).unwrap();
        // 0.5 + 0.0 (no date bonus beyond 5 days) = 0.5
        assert_eq!(score, Decimal::from_str("0.5000").unwrap());
    }

    #[test]
    fn cc_deterministic_output() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let strategy = CreditCardStrategy;
        let sl = make_cc_txn(-4500, d, Some(d), Some(d), Some("AMAZON"), None, true);
        let pt = make_cc_txn(-4500, d, Some(d), Some(d), Some("AMAZON"), None, false);

        let run1 = strategy.score(&sl, &pt);
        let run2 = strategy.score(&sl, &pt);
        assert_eq!(run1, run2);
    }
}
