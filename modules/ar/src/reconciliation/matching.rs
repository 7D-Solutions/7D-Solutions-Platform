//! Deterministic matching algorithm (pure logic — no DB).
//!
//! This module contains the core matching heuristics that pair unmatched
//! payments to open invoices. The algorithm is pure: it takes sorted slices
//! and returns proposed matches and exceptions without any I/O.

use crate::events::ReconExceptionKind;
use std::collections::HashMap;

use super::{OpenInvoice, ProposedException, ProposedMatch, UnmatchedPayment};

/// Match payments to invoices using deterministic heuristic rules.
///
/// **Determinism guarantee**: Both input slices must be sorted by
/// (ar_customer_id, amount_cents, id). The matching algorithm processes
/// payments in order and greedily assigns the best match. Because both
/// sides are sorted identically, the same inputs always yield the same
/// output.
///
/// **Rules (evaluated in priority order):**
/// 1. Exact match: same customer, same amount, same currency → confidence 1.0
/// 2. Reference match: payment reference_id matches invoice tilled_invoice_id → confidence 0.95
///
/// Ties (equal confidence for multiple invoices) → AmbiguousMatch exception.
/// No match at all → UnmatchedPayment exception.
pub(crate) fn match_payments_to_invoices(
    payments: &[UnmatchedPayment],
    invoices: &[OpenInvoice],
) -> (Vec<ProposedMatch>, Vec<ProposedException>) {
    let mut matches = Vec::new();
    let mut exceptions = Vec::new();
    // Track which invoices have already been matched (by invoice_id).
    let mut matched_invoice_ids: std::collections::HashSet<i32> = std::collections::HashSet::new();

    // Build invoice lookup by customer for efficient searching.
    let mut invoices_by_customer: HashMap<i32, Vec<&OpenInvoice>> = HashMap::new();
    for inv in invoices {
        invoices_by_customer
            .entry(inv.ar_customer_id)
            .or_default()
            .push(inv);
    }

    for payment in payments {
        let mut candidates: Vec<(&OpenInvoice, f64, &str)> = Vec::new();

        // Rule 1: Exact match — same customer + same amount + same currency.
        if let Some(customer_invoices) = invoices_by_customer.get(&payment.ar_customer_id) {
            for inv in customer_invoices {
                if matched_invoice_ids.contains(&inv.invoice_id) {
                    continue;
                }
                if inv.amount_cents == payment.amount_cents
                    && inv.currency.eq_ignore_ascii_case(&payment.currency)
                {
                    candidates.push((inv, 1.0, "exact"));
                }
            }
        }

        // Rule 2: Reference match — payment reference_id matches invoice tilled_invoice_id.
        if candidates.is_empty() {
            if let Some(ref ref_id) = payment.reference_id {
                for inv in invoices {
                    if matched_invoice_ids.contains(&inv.invoice_id) {
                        continue;
                    }
                    if inv.tilled_invoice_id == *ref_id
                        && inv.currency.eq_ignore_ascii_case(&payment.currency)
                    {
                        candidates.push((inv, 0.95, "reference"));
                    }
                }
            }
        }

        match candidates.len() {
            0 => {
                // No match found → UnmatchedPayment exception.
                exceptions.push(ProposedException {
                    payment_id: Some(payment.charge_id.to_string()),
                    invoice_id: None,
                    exception_kind: ReconExceptionKind::UnmatchedPayment,
                    description: format!(
                        "No matching invoice found for payment {} (amount: {}, currency: {})",
                        payment.charge_id, payment.amount_cents, payment.currency
                    ),
                    amount_minor: Some(payment.amount_cents as i64),
                    currency: Some(payment.currency.clone()),
                });
            }
            1 => {
                // Single best match — apply it.
                let (inv, score, method) = candidates[0];
                matched_invoice_ids.insert(inv.invoice_id);
                matches.push(ProposedMatch {
                    payment: payment.clone(),
                    invoice: inv.clone(),
                    matched_amount_minor: payment.amount_cents as i64,
                    confidence_score: score,
                    match_method: method.to_string(),
                });
            }
            _ => {
                // Multiple candidates with equal confidence → ambiguous.
                // Check if all candidates share the same top score.
                let top_score = candidates
                    .iter()
                    .map(|(_, s, _)| *s)
                    .fold(f64::NEG_INFINITY, f64::max);
                let top_candidates: Vec<_> = candidates
                    .iter()
                    .filter(|(_, s, _)| (*s - top_score).abs() < f64::EPSILON)
                    .collect();

                if top_candidates.len() == 1 {
                    // One clear winner at the top score.
                    let (inv, score, method) = top_candidates[0];
                    matched_invoice_ids.insert(inv.invoice_id);
                    matches.push(ProposedMatch {
                        payment: payment.clone(),
                        invoice: (*inv).clone(),
                        matched_amount_minor: payment.amount_cents as i64,
                        confidence_score: *score,
                        match_method: method.to_string(),
                    });
                } else {
                    // True ambiguity — raise exception.
                    let invoice_ids: Vec<String> = top_candidates
                        .iter()
                        .map(|(inv, _, _)| inv.invoice_id.to_string())
                        .collect();
                    exceptions.push(ProposedException {
                        payment_id: Some(payment.charge_id.to_string()),
                        invoice_id: None,
                        exception_kind: ReconExceptionKind::AmbiguousMatch,
                        description: format!(
                            "Payment {} matches multiple invoices with equal confidence: [{}]",
                            payment.charge_id,
                            invoice_ids.join(", ")
                        ),
                        amount_minor: Some(payment.amount_cents as i64),
                        currency: Some(payment.currency.clone()),
                    });
                }
            }
        }
    }

    (matches, exceptions)
}

/// Convert ReconExceptionKind to the DB string representation.
pub fn exception_kind_to_str(kind: &ReconExceptionKind) -> &'static str {
    match kind {
        ReconExceptionKind::UnmatchedPayment => "unmatched_payment",
        ReconExceptionKind::UnmatchedInvoice => "unmatched_invoice",
        ReconExceptionKind::AmountMismatch => "amount_mismatch",
        ReconExceptionKind::AmbiguousMatch => "ambiguous_match",
        ReconExceptionKind::DuplicateReference => "duplicate_reference",
    }
}

// ============================================================================
// Unit tests (pure logic — no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_payment(
        id: i32,
        customer: i32,
        amount: i64,
        currency: &str,
        ref_id: Option<&str>,
    ) -> UnmatchedPayment {
        UnmatchedPayment {
            charge_id: id,
            ar_customer_id: customer,
            amount_cents: amount,
            currency: currency.to_string(),
            reference_id: ref_id.map(|s| s.to_string()),
        }
    }

    fn make_invoice(
        id: i32,
        customer: i32,
        amount: i64,
        currency: &str,
        tilled_id: &str,
    ) -> OpenInvoice {
        OpenInvoice {
            invoice_id: id,
            ar_customer_id: customer,
            amount_cents: amount,
            currency: currency.to_string(),
            tilled_invoice_id: tilled_id.to_string(),
        }
    }

    #[test]
    fn exact_match_single_payment_single_invoice() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![make_invoice(10, 100, 5000, "usd", "inv-10")];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 1);
        assert_eq!(exceptions.len(), 0);
        assert_eq!(matches[0].payment.charge_id, 1);
        assert_eq!(matches[0].invoice.invoice_id, 10);
        assert_eq!(matches[0].confidence_score, 1.0);
        assert_eq!(matches[0].match_method, "exact");
    }

    #[test]
    fn unmatched_payment_raises_exception() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![make_invoice(10, 200, 5000, "usd", "inv-10")]; // different customer

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(
            exceptions[0].exception_kind,
            ReconExceptionKind::UnmatchedPayment
        );
        assert_eq!(exceptions[0].payment_id, Some("1".to_string()));
    }

    #[test]
    fn reference_match_when_no_exact_match() {
        let payments = vec![make_payment(1, 100, 5000, "usd", Some("inv-10"))];
        let invoices = vec![make_invoice(10, 200, 3000, "usd", "inv-10")]; // different customer & amount

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 1);
        assert_eq!(exceptions.len(), 0);
        assert_eq!(matches[0].confidence_score, 0.95);
        assert_eq!(matches[0].match_method, "reference");
    }

    #[test]
    fn ambiguous_match_raises_exception() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![
            make_invoice(10, 100, 5000, "usd", "inv-10"),
            make_invoice(11, 100, 5000, "usd", "inv-11"),
        ];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(
            exceptions[0].exception_kind,
            ReconExceptionKind::AmbiguousMatch
        );
    }

    #[test]
    fn multiple_payments_matched_correctly() {
        let payments = vec![
            make_payment(1, 100, 5000, "usd", None),
            make_payment(2, 200, 3000, "usd", None),
        ];
        let invoices = vec![
            make_invoice(10, 100, 5000, "usd", "inv-10"),
            make_invoice(11, 200, 3000, "usd", "inv-11"),
        ];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 2);
        assert_eq!(exceptions.len(), 0);
        // First payment matches first invoice
        assert_eq!(matches[0].payment.charge_id, 1);
        assert_eq!(matches[0].invoice.invoice_id, 10);
        // Second payment matches second invoice
        assert_eq!(matches[1].payment.charge_id, 2);
        assert_eq!(matches[1].invoice.invoice_id, 11);
    }

    #[test]
    fn currency_mismatch_prevents_match() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![make_invoice(10, 100, 5000, "eur", "inv-10")];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(
            exceptions[0].exception_kind,
            ReconExceptionKind::UnmatchedPayment
        );
    }

    #[test]
    fn determinism_across_runs() {
        let payments = vec![
            make_payment(1, 100, 5000, "usd", None),
            make_payment(2, 200, 3000, "usd", None),
            make_payment(3, 300, 7000, "usd", None),
        ];
        let invoices = vec![
            make_invoice(10, 100, 5000, "usd", "inv-10"),
            make_invoice(11, 200, 3000, "usd", "inv-11"),
        ];

        // Run twice — must produce identical results.
        let (matches1, exceptions1) = match_payments_to_invoices(&payments, &invoices);
        let (matches2, exceptions2) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches1.len(), matches2.len());
        assert_eq!(exceptions1.len(), exceptions2.len());
        for (m1, m2) in matches1.iter().zip(matches2.iter()) {
            assert_eq!(m1.payment.charge_id, m2.payment.charge_id);
            assert_eq!(m1.invoice.invoice_id, m2.invoice.invoice_id);
            assert_eq!(m1.confidence_score, m2.confidence_score);
            assert_eq!(m1.match_method, m2.match_method);
        }
    }

    #[test]
    fn empty_inputs_produce_no_output() {
        let (matches, exceptions) = match_payments_to_invoices(&[], &[]);
        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 0);
    }

    #[test]
    fn already_matched_invoice_not_reused() {
        // Two payments, same amount, same customer — but only one invoice available.
        // First payment matches; second becomes unmatched.
        let payments = vec![
            make_payment(1, 100, 5000, "usd", None),
            make_payment(2, 100, 5000, "usd", None),
        ];
        let invoices = vec![make_invoice(10, 100, 5000, "usd", "inv-10")];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 1);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(matches[0].payment.charge_id, 1); // first payment wins
        assert_eq!(
            exceptions[0].exception_kind,
            ReconExceptionKind::UnmatchedPayment
        );
        assert_eq!(exceptions[0].payment_id, Some("2".to_string()));
    }

    #[test]
    fn exception_kind_to_str_roundtrip() {
        assert_eq!(
            exception_kind_to_str(&ReconExceptionKind::UnmatchedPayment),
            "unmatched_payment"
        );
        assert_eq!(
            exception_kind_to_str(&ReconExceptionKind::UnmatchedInvoice),
            "unmatched_invoice"
        );
        assert_eq!(
            exception_kind_to_str(&ReconExceptionKind::AmountMismatch),
            "amount_mismatch"
        );
        assert_eq!(
            exception_kind_to_str(&ReconExceptionKind::AmbiguousMatch),
            "ambiguous_match"
        );
        assert_eq!(
            exception_kind_to_str(&ReconExceptionKind::DuplicateReference),
            "duplicate_reference"
        );
    }
}
