//! Auto-match heuristics for reconciliation.
//!
//! Matches statement lines (CSV-imported, `statement_id IS NOT NULL`) to
//! payment-event transactions (`statement_id IS NULL`) using deterministic
//! scoring. Same inputs always produce same matches.
//!
//! Supports pluggable strategies via `MatchStrategy` trait — bank (default)
//! and credit card strategies produce different scoring for the same pair.

use rust_decimal::Decimal;

use super::models::UnmatchedTxn;
use super::strategies::MatchStrategy;

/// A candidate match between a statement line and a bank transaction.
#[derive(Debug, Clone)]
pub struct CandidateMatch {
    pub statement_line: UnmatchedTxn,
    pub bank_transaction: UnmatchedTxn,
    pub confidence: Decimal,
}

/// Run deterministic auto-match with the given strategy.
///
/// 1. For each (statement_line, payment_txn) pair, call `strategy.score()`.
/// 2. Skip pairs where the strategy returns `None` (mandatory criteria failed).
/// 3. Sort all candidates by confidence descending, then by IDs for stability.
/// 4. Greedily assign — each line and transaction used at most once.
pub fn auto_match_with_strategy(
    statement_lines: &[UnmatchedTxn],
    payment_txns: &[UnmatchedTxn],
    strategy: &dyn MatchStrategy,
) -> Vec<CandidateMatch> {
    let mut candidates: Vec<CandidateMatch> = Vec::new();

    for sl in statement_lines {
        for pt in payment_txns {
            if let Some(confidence) = strategy.score(sl, pt) {
                candidates.push(CandidateMatch {
                    statement_line: sl.clone(),
                    bank_transaction: pt.clone(),
                    confidence,
                });
            }
        }
    }

    // Sort: highest confidence first, then deterministic tie-breaking by IDs
    candidates.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| a.statement_line.id.cmp(&b.statement_line.id))
            .then_with(|| a.bank_transaction.id.cmp(&b.bank_transaction.id))
    });

    // Greedy assignment: each ID used at most once
    let mut used_lines = std::collections::HashSet::new();
    let mut used_txns = std::collections::HashSet::new();
    let mut result = Vec::new();

    for c in candidates {
        if used_lines.contains(&c.statement_line.id) || used_txns.contains(&c.bank_transaction.id) {
            continue;
        }
        used_lines.insert(c.statement_line.id);
        used_txns.insert(c.bank_transaction.id);
        result.push(c);
    }

    result
}

/// Convenience: auto-match using the default bank strategy.
pub fn auto_match(
    statement_lines: &[UnmatchedTxn],
    payment_txns: &[UnmatchedTxn],
) -> Vec<CandidateMatch> {
    auto_match_with_strategy(statement_lines, payment_txns, &BankStrategy)
}

// ============================================================================
// Bank strategy (default)
// ============================================================================

/// Default bank reconciliation strategy — exact amount + date proximity +
/// reference similarity.
pub struct BankStrategy;

impl MatchStrategy for BankStrategy {
    fn score(&self, sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Option<Decimal> {
        if sl.amount_minor != pt.amount_minor {
            return None;
        }
        if sl.currency != pt.currency {
            return None;
        }
        Some(score_bank(sl, pt))
    }
}

/// Score a candidate pair for bank recon. Amount match is a prerequisite
/// (caller filters). Returns confidence in [0.5000, 1.0000].
fn score_bank(sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Decimal {
    let mut score = Decimal::new(5000, 4); // base: amount match

    // Date proximity bonus (up to +0.3)
    let day_diff = (sl.transaction_date - pt.transaction_date)
        .num_days()
        .unsigned_abs();
    let date_bonus = match day_diff {
        0 => Decimal::new(3000, 4),
        1 => Decimal::new(2000, 4),
        2 => Decimal::new(1000, 4),
        3 => Decimal::new(500, 4),
        _ => Decimal::ZERO,
    };
    score += date_bonus;

    // Reference match bonus (up to +0.2)
    let ref_bonus = reference_similarity(sl.reference.as_deref(), pt.reference.as_deref());
    score += ref_bonus;

    score
}

/// Compare references: exact match → 0.2, substring containment → 0.1, else 0.
fn reference_similarity(a: Option<&str>, b: Option<&str>) -> Decimal {
    match (a, b) {
        (Some(ra), Some(rb)) => {
            let ra = ra.trim().to_lowercase();
            let rb = rb.trim().to_lowercase();
            if ra == rb && !ra.is_empty() {
                Decimal::new(2000, 4)
            } else if (!ra.is_empty() && rb.contains(&ra)) || (!rb.is_empty() && ra.contains(&rb)) {
                Decimal::new(1000, 4)
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

    fn make_txn(
        amount: i64,
        date: NaiveDate,
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
            auth_date: None,
            settle_date: None,
            merchant_name: None,
        }
    }

    #[test]
    fn exact_match_scores_highest() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let sl = make_txn(-450, d, Some("TXN001"), true);
        let pt = make_txn(-450, d, Some("TXN001"), false);

        let matches = auto_match(&[sl], &[pt]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].confidence, Decimal::from_str("1.0000").unwrap());
    }

    #[test]
    fn amount_mismatch_produces_no_match() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let sl = make_txn(-450, d, Some("TXN001"), true);
        let pt = make_txn(-451, d, Some("TXN001"), false);

        let matches = auto_match(&[sl], &[pt]);
        assert!(matches.is_empty());
    }

    #[test]
    fn stable_output_for_same_input() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let sl = make_txn(-450, d, Some("TXN001"), true);
        let pt = make_txn(-450, d, Some("TXN001"), false);

        let run1 = auto_match(&[sl.clone()], &[pt.clone()]);
        let run2 = auto_match(&[sl], &[pt]);
        assert_eq!(run1.len(), run2.len());
        assert_eq!(run1[0].confidence, run2[0].confidence);
        assert_eq!(run1[0].statement_line.id, run2[0].statement_line.id);
    }

    #[test]
    fn greedy_one_to_one() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let sl1 = make_txn(-100, d, None, true);
        let sl2 = make_txn(-100, d, None, true);
        let pt1 = make_txn(-100, d, None, false);

        let matches = auto_match(&[sl1, sl2], &[pt1]);
        assert_eq!(matches.len(), 1, "only one txn available → one match");
    }

    #[test]
    fn strategy_dispatches_correctly() {
        use super::super::strategies::credit_card::CreditCardStrategy;

        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let sl = UnmatchedTxn {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            transaction_date: d,
            amount_minor: -2500,
            currency: "USD".to_string(),
            description: None,
            reference: None,
            statement_id: Some(Uuid::new_v4()),
            auth_date: None,
            settle_date: None,
            merchant_name: Some("STARBUCKS".to_string()),
        };
        let pt = UnmatchedTxn {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            transaction_date: d,
            amount_minor: -2500,
            currency: "USD".to_string(),
            description: None,
            reference: None,
            statement_id: None,
            auth_date: Some(d),
            settle_date: Some(d),
            merchant_name: Some("STARBUCKS".to_string()),
        };

        let bank_result = auto_match_with_strategy(&[sl.clone()], &[pt.clone()], &BankStrategy);
        let cc_result = auto_match_with_strategy(&[sl.clone()], &[pt.clone()], &CreditCardStrategy);

        assert_eq!(bank_result.len(), 1);
        assert_eq!(cc_result.len(), 1);
        // CC strategy uses merchant matching so confidence differs from bank
        assert_ne!(bank_result[0].confidence, cc_result[0].confidence);
    }
}
