//! Auto-match heuristics for reconciliation.
//!
//! Matches statement lines (CSV-imported, `statement_id IS NOT NULL`) to
//! payment-event transactions (`statement_id IS NULL`) using deterministic
//! scoring. Same inputs always produce same matches.
//!
//! Supports pluggable strategies via `MatchStrategy` trait — bank (default)
//! and credit card strategies produce different scoring for the same pair.

use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

use super::models::UnmatchedTxn;
use super::strategies::MatchStrategy;

/// A candidate match between a statement line and a bank transaction.
#[derive(Debug, Clone)]
pub struct CandidateMatch {
    pub statement_line: UnmatchedTxn,
    pub bank_transaction: UnmatchedTxn,
    pub confidence: Decimal,
}

#[derive(Debug, Clone, Copy)]
struct CandidateIndex {
    statement_line_idx: usize,
    bank_transaction_idx: usize,
    confidence: Decimal,
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
    let mut candidates: Vec<CandidateIndex> = Vec::new();
    let payment_buckets = strategy
        .requires_exact_amount_currency()
        .then(|| bucket_payment_txns(payment_txns));

    for (statement_line_idx, sl) in statement_lines.iter().enumerate() {
        if let Some(payment_buckets) = payment_buckets.as_ref() {
            if let Some(payment_indices) =
                payment_buckets.get(&(sl.amount_minor, sl.currency.as_str()))
            {
                for &bank_transaction_idx in payment_indices {
                    let pt = &payment_txns[bank_transaction_idx];
                    if let Some(confidence) = strategy.score(sl, pt) {
                        candidates.push(CandidateIndex {
                            statement_line_idx,
                            bank_transaction_idx,
                            confidence,
                        });
                    }
                }
            }
        } else {
            for (bank_transaction_idx, pt) in payment_txns.iter().enumerate() {
                if let Some(confidence) = strategy.score(sl, pt) {
                    candidates.push(CandidateIndex {
                        statement_line_idx,
                        bank_transaction_idx,
                        confidence,
                    });
                }
            }
        }
    }

    // Sort: highest confidence first, then deterministic tie-breaking by IDs
    candidates.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| {
                statement_lines[a.statement_line_idx]
                    .id
                    .cmp(&statement_lines[b.statement_line_idx].id)
            })
            .then_with(|| {
                payment_txns[a.bank_transaction_idx]
                    .id
                    .cmp(&payment_txns[b.bank_transaction_idx].id)
            })
    });

    // Greedy assignment: each ID used at most once
    let mut used_lines = HashSet::with_capacity(statement_lines.len());
    let mut used_txns = HashSet::with_capacity(payment_txns.len());
    let mut result = Vec::with_capacity(statement_lines.len().min(payment_txns.len()));

    for c in candidates {
        let statement_line = &statement_lines[c.statement_line_idx];
        let bank_transaction = &payment_txns[c.bank_transaction_idx];

        if used_lines.contains(&statement_line.id) || used_txns.contains(&bank_transaction.id) {
            continue;
        }
        used_lines.insert(statement_line.id);
        used_txns.insert(bank_transaction.id);
        result.push(CandidateMatch {
            statement_line: statement_line.clone(),
            bank_transaction: bank_transaction.clone(),
            confidence: c.confidence,
        });
    }

    result
}

fn bucket_payment_txns(payment_txns: &[UnmatchedTxn]) -> HashMap<(i64, &str), Vec<usize>> {
    let mut buckets = HashMap::with_capacity(payment_txns.len());

    for (idx, txn) in payment_txns.iter().enumerate() {
        buckets
            .entry((txn.amount_minor, txn.currency.as_str()))
            .or_insert_with(Vec::new)
            .push(idx);
    }

    buckets
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
    use std::time::Instant;
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
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
        let sl = make_txn(-450, d, Some("TXN001"), true);
        let pt = make_txn(-450, d, Some("TXN001"), false);

        let matches = auto_match(&[sl], &[pt]);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].confidence,
            Decimal::from_str("1.0000").expect("valid decimal")
        );
    }

    #[test]
    fn amount_mismatch_produces_no_match() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
        let sl = make_txn(-450, d, Some("TXN001"), true);
        let pt = make_txn(-451, d, Some("TXN001"), false);

        let matches = auto_match(&[sl], &[pt]);
        assert!(matches.is_empty());
    }

    #[test]
    fn stable_output_for_same_input() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
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
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
        let sl1 = make_txn(-100, d, None, true);
        let sl2 = make_txn(-100, d, None, true);
        let pt1 = make_txn(-100, d, None, false);

        let matches = auto_match(&[sl1, sl2], &[pt1]);
        assert_eq!(matches.len(), 1, "only one txn available → one match");
    }

    #[test]
    fn strategy_dispatches_correctly() {
        use super::super::strategies::credit_card::CreditCardStrategy;

        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
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

    fn legacy_auto_match_with_strategy(
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

        candidates.sort_by(|a, b| {
            b.confidence
                .cmp(&a.confidence)
                .then_with(|| a.statement_line.id.cmp(&b.statement_line.id))
                .then_with(|| a.bank_transaction.id.cmp(&b.bank_transaction.id))
        });

        let mut used_lines = HashSet::new();
        let mut used_txns = HashSet::new();
        let mut result = Vec::new();

        for c in candidates {
            if used_lines.contains(&c.statement_line.id)
                || used_txns.contains(&c.bank_transaction.id)
            {
                continue;
            }
            used_lines.insert(c.statement_line.id);
            used_txns.insert(c.bank_transaction.id);
            result.push(c);
        }

        result
    }

    #[test]
    fn optimized_engine_matches_legacy_output() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
        let statement_lines = vec![
            make_txn(-450, d, Some("TXN001"), true),
            make_txn(-451, d, Some("TXN002"), true),
            make_txn(-452, d, Some("TXN003"), true),
        ];
        let payment_txns = vec![
            make_txn(-452, d, Some("TXN003"), false),
            make_txn(-450, d, Some("TXN001"), false),
            make_txn(-451, d, Some("TXN002"), false),
        ];

        let legacy =
            legacy_auto_match_with_strategy(&statement_lines, &payment_txns, &BankStrategy);
        let optimized = auto_match_with_strategy(&statement_lines, &payment_txns, &BankStrategy);

        assert_eq!(legacy.len(), optimized.len());
        for (lhs, rhs) in legacy.iter().zip(optimized.iter()) {
            assert_eq!(lhs.statement_line.id, rhs.statement_line.id);
            assert_eq!(lhs.bank_transaction.id, rhs.bank_transaction.id);
            assert_eq!(lhs.confidence, rhs.confidence);
        }
    }

    #[test]
    #[ignore]
    fn benchmark_bucketed_matching_against_legacy() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid test date");
        let statement_lines: Vec<UnmatchedTxn> = (0..2_000)
            .map(|idx| make_txn(-(10_000 + idx as i64), d, Some("bench"), true))
            .collect();
        let payment_txns: Vec<UnmatchedTxn> = (0..2_000)
            .map(|idx| make_txn(-(10_000 + idx as i64), d, Some("bench"), false))
            .collect();

        let legacy_start = Instant::now();
        let legacy =
            legacy_auto_match_with_strategy(&statement_lines, &payment_txns, &BankStrategy);
        let legacy_elapsed = legacy_start.elapsed();

        let optimized_start = Instant::now();
        let optimized = auto_match_with_strategy(&statement_lines, &payment_txns, &BankStrategy);
        let optimized_elapsed = optimized_start.elapsed();

        assert_eq!(legacy.len(), optimized.len());

        println!(
            "legacy={:?} optimized={:?} speedup={:.2}x",
            legacy_elapsed,
            optimized_elapsed,
            legacy_elapsed.as_secs_f64() / optimized_elapsed.as_secs_f64()
        );
    }
}
