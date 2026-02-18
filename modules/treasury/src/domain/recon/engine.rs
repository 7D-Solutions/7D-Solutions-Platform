//! Auto-match heuristics for bank reconciliation.
//!
//! Matches statement lines (CSV-imported, `statement_id IS NOT NULL`) to
//! payment-event transactions (`statement_id IS NULL`) using deterministic
//! scoring. Same inputs always produce same matches.

use rust_decimal::Decimal;
use std::str::FromStr;

use super::models::UnmatchedTxn;

/// A candidate match between a statement line and a bank transaction.
#[derive(Debug, Clone)]
pub struct CandidateMatch {
    pub statement_line: UnmatchedTxn,
    pub bank_transaction: UnmatchedTxn,
    pub confidence: Decimal,
}

/// Run deterministic auto-match: pair statement lines to payment transactions.
///
/// Strategy (greedy, highest-confidence-first):
/// 1. For each statement line, score every unmatched payment transaction.
/// 2. Only keep pairs where amount matches exactly (mandatory).
/// 3. Score date proximity and reference similarity as bonus confidence.
/// 4. Sort all candidates by confidence descending, then by IDs for stability.
/// 5. Greedily assign — each line and transaction used at most once.
pub fn auto_match(
    statement_lines: &[UnmatchedTxn],
    payment_txns: &[UnmatchedTxn],
) -> Vec<CandidateMatch> {
    let mut candidates: Vec<CandidateMatch> = Vec::new();

    for sl in statement_lines {
        for pt in payment_txns {
            if sl.amount_minor != pt.amount_minor {
                continue;
            }
            if sl.currency != pt.currency {
                continue;
            }
            let confidence = score(sl, pt);
            candidates.push(CandidateMatch {
                statement_line: sl.clone(),
                bank_transaction: pt.clone(),
                confidence,
            });
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
        if used_lines.contains(&c.statement_line.id) || used_txns.contains(&c.bank_transaction.id)
        {
            continue;
        }
        used_lines.insert(c.statement_line.id);
        used_txns.insert(c.bank_transaction.id);
        result.push(c);
    }

    result
}

/// Score a candidate pair. Amount match is a prerequisite (caller filters).
/// Returns confidence in [0.5000, 1.0000].
fn score(sl: &UnmatchedTxn, pt: &UnmatchedTxn) -> Decimal {
    let mut score = Decimal::from_str("0.5000").unwrap(); // base: amount match

    // Date proximity bonus (up to +0.3)
    let day_diff = (sl.transaction_date - pt.transaction_date).num_days().unsigned_abs();
    let date_bonus = match day_diff {
        0 => Decimal::from_str("0.3000").unwrap(),
        1 => Decimal::from_str("0.2000").unwrap(),
        2 => Decimal::from_str("0.1000").unwrap(),
        3 => Decimal::from_str("0.0500").unwrap(),
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
                Decimal::from_str("0.2000").unwrap()
            } else if (!ra.is_empty() && rb.contains(&ra))
                || (!rb.is_empty() && ra.contains(&rb))
            {
                Decimal::from_str("0.1000").unwrap()
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
        }
    }

    #[test]
    fn exact_match_scores_highest() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let sl = make_txn(-450, d, Some("TXN001"), true);
        let pt = make_txn(-450, d, Some("TXN001"), false);

        let matches = auto_match(&[sl], &[pt]);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].confidence,
            Decimal::from_str("1.0000").unwrap()
        );
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
}
