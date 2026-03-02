//! Currency Conversion Utilities (Phase 23a, bd-24y)
//!
//! Deterministic conversion of monetary amounts between transaction and
//! reporting currencies using a specific FX rate snapshot.
//!
//! ## Design Principles
//!
//! 1. **Convert at posting time** — never at query time. Both transaction
//!    and reporting amounts are stored on journal lines.
//! 2. **Rate reference is mandatory** — every conversion records which rate
//!    snapshot was used (rate_id + effective_at) for audit trail.
//! 3. **Banker's rounding** (round half to even) for all conversions.
//! 4. **Sign preservation** — debits stay positive, credits stay positive.
//!    The sign of the converted amount always matches the input.
//! 5. **Balance invariant** — sum of converted debits == sum of converted
//!    credits (enforced by a pennies-off adjustment on the last line).

// Re-export types so existing callers (e.g. fx_revaluation_service) continue
// to import from this module without changes.
pub use super::currency_types::{ConversionError, ConvertedAmount, ConvertedLine, RateSnapshot};

// ============================================================================
// Core conversion functions
// ============================================================================

/// Convert a single amount from transaction to reporting currency.
///
/// Uses banker's rounding (round half to even) for deterministic results.
///
/// # Arguments
/// * `amount_minor` - Amount in transaction currency minor units (must be >= 0)
/// * `rate` - The conversion rate snapshot
/// * `transaction_currency` - ISO 4217 code of the source currency
/// * `reporting_currency` - ISO 4217 code of the target currency
///
/// # Returns
/// The converted amount in reporting currency minor units.
///
/// # Rounding
/// Uses banker's rounding: `round(amount * rate)` where ties go to the
/// nearest even number. This minimizes systematic rounding bias.
pub fn convert_amount(
    amount_minor: i64,
    rate: &RateSnapshot,
    transaction_currency: &str,
    reporting_currency: &str,
) -> Result<ConvertedAmount, ConversionError> {
    if amount_minor < 0 {
        return Err(ConversionError::NegativeAmount(amount_minor));
    }
    if !rate.rate.is_finite() || rate.rate <= 0.0 {
        return Err(ConversionError::InvalidRate(format!("{}", rate.rate)));
    }

    // Determine which direction to convert
    let effective_rate = resolve_rate(rate, transaction_currency, reporting_currency)?;

    let reporting = bankers_round(amount_minor as f64 * effective_rate);

    Ok(ConvertedAmount {
        transaction_amount_minor: amount_minor,
        reporting_amount_minor: reporting,
    })
}

/// Convert a batch of journal lines and enforce the balance invariant.
///
/// After individual line conversion, checks that total reporting debits
/// equal total reporting credits. If they differ by a rounding residual
/// (at most 1 minor unit per line), adjusts the largest line to restore
/// balance.
///
/// # Arguments
/// * `lines` - Pairs of (debit_minor, credit_minor) in transaction currency
/// * `rate` - The rate snapshot to use
/// * `transaction_currency` - Source currency
/// * `reporting_currency` - Target currency
///
/// # Returns
/// Converted lines with the balance invariant guaranteed.
pub fn convert_journal_lines(
    lines: &[(i64, i64)],
    rate: &RateSnapshot,
    transaction_currency: &str,
    reporting_currency: &str,
) -> Result<Vec<ConvertedLine>, ConversionError> {
    if !rate.rate.is_finite() || rate.rate <= 0.0 {
        return Err(ConversionError::InvalidRate(format!("{}", rate.rate)));
    }

    let effective_rate = resolve_rate(rate, transaction_currency, reporting_currency)?;

    let mut converted: Vec<ConvertedLine> = lines
        .iter()
        .map(|&(debit, credit)| {
            if debit < 0 || credit < 0 {
                return Err(ConversionError::NegativeAmount(debit.min(credit)));
            }
            Ok(ConvertedLine {
                txn_debit_minor: debit,
                txn_credit_minor: credit,
                rpt_debit_minor: bankers_round(debit as f64 * effective_rate),
                rpt_credit_minor: bankers_round(credit as f64 * effective_rate),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Enforce balance invariant: sum(rpt_debits) == sum(rpt_credits)
    let total_rpt_debits: i64 = converted.iter().map(|l| l.rpt_debit_minor).sum();
    let total_rpt_credits: i64 = converted.iter().map(|l| l.rpt_credit_minor).sum();
    let residual = total_rpt_debits - total_rpt_credits;

    if residual != 0 {
        // Adjust the largest line to absorb the residual
        // This is standard practice: the "pennies-off" adjustment
        let max_idx = find_largest_line_index(&converted);
        if residual > 0 {
            // Debits exceed credits — increase credits on largest line
            converted[max_idx].rpt_credit_minor += residual;
        } else {
            // Credits exceed debits — increase debits on largest line
            converted[max_idx].rpt_debit_minor += -residual;
        }
    }

    // Verify invariant holds after adjustment
    let final_debits: i64 = converted.iter().map(|l| l.rpt_debit_minor).sum();
    let final_credits: i64 = converted.iter().map(|l| l.rpt_credit_minor).sum();
    if final_debits != final_credits {
        return Err(ConversionError::BalanceInvariant {
            debits: final_debits,
            credits: final_credits,
        });
    }

    Ok(converted)
}

/// Check if a conversion is needed (currencies differ).
pub fn requires_conversion(transaction_currency: &str, reporting_currency: &str) -> bool {
    transaction_currency.to_uppercase() != reporting_currency.to_uppercase()
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Resolve the effective rate for a given currency direction.
///
/// If the rate is base/quote matching transaction→reporting, use rate directly.
/// If inverted (quote→base), use inverse_rate.
fn resolve_rate(
    snapshot: &RateSnapshot,
    transaction_currency: &str,
    reporting_currency: &str,
) -> Result<f64, ConversionError> {
    let txn = transaction_currency.to_uppercase();
    let rpt = reporting_currency.to_uppercase();
    let base = snapshot.base_currency.to_uppercase();
    let quote = snapshot.quote_currency.to_uppercase();

    if txn == base && rpt == quote {
        // Direct: 1 base = rate quote → multiply by rate
        Ok(snapshot.rate)
    } else if txn == quote && rpt == base {
        // Inverse: 1 quote = inverse_rate base → multiply by inverse_rate
        Ok(snapshot.inverse_rate)
    } else {
        Err(ConversionError::CurrencyMismatch {
            rate_base: base,
            rate_quote: quote,
            from: txn,
            to: rpt,
        })
    }
}

/// Banker's rounding (round half to even).
///
/// For f64 → i64 conversion of monetary amounts.
/// When the fractional part is exactly 0.5, rounds to the nearest even integer.
fn bankers_round(value: f64) -> i64 {
    let floor = value.floor();
    let frac = value - floor;
    let floor_i = floor as i64;

    if (frac - 0.5).abs() < 1e-9 {
        // Exactly 0.5 — round to even
        if floor_i % 2 == 0 {
            floor_i
        } else {
            floor_i + 1
        }
    } else {
        value.round() as i64
    }
}

/// Find the index of the line with the largest absolute amount.
///
/// Used for pennies-off adjustment — we adjust the largest line to minimize
/// the relative impact of the rounding correction.
fn find_largest_line_index(lines: &[ConvertedLine]) -> usize {
    lines
        .iter()
        .enumerate()
        .max_by_key(|(_, l)| l.rpt_debit_minor.max(l.rpt_credit_minor))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn eur_usd_rate(rate: f64) -> RateSnapshot {
        RateSnapshot {
            rate_id: Uuid::new_v4(),
            rate,
            inverse_rate: 1.0 / rate,
            effective_at: Utc::now(),
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
        }
    }

    #[test]
    fn convert_zero_amount() {
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(0, &rate, "EUR", "USD").unwrap();
        assert_eq!(result.reporting_amount_minor, 0);
    }

    #[test]
    fn convert_simple_amount() {
        // 1000.00 EUR at 1.085 = 1085.00 USD = 108500 minor
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(100000, &rate, "EUR", "USD").unwrap();
        assert_eq!(result.reporting_amount_minor, 108500);
    }

    #[test]
    fn convert_inverse_direction() {
        // 1085.00 USD at inverse(1.085) = 1000.00 EUR = 100000 minor
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(108500, &rate, "USD", "EUR").unwrap();
        assert_eq!(result.reporting_amount_minor, 100000);
    }

    #[test]
    fn convert_with_rounding() {
        // 33.33 EUR at 1.085 = 36.16305 → rounds to 3616 minor
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(3333, &rate, "EUR", "USD").unwrap();
        assert_eq!(result.reporting_amount_minor, 3616);
    }

    #[test]
    fn convert_rejects_negative_amount() {
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(-100, &rate, "EUR", "USD");
        assert!(matches!(result, Err(ConversionError::NegativeAmount(-100))));
    }

    #[test]
    fn convert_rejects_zero_rate() {
        let mut rate = eur_usd_rate(1.085);
        rate.rate = 0.0;
        let result = convert_amount(100, &rate, "EUR", "USD");
        assert!(matches!(result, Err(ConversionError::InvalidRate(_))));
    }

    #[test]
    fn convert_rejects_negative_rate() {
        let mut rate = eur_usd_rate(1.085);
        rate.rate = -1.5;
        let result = convert_amount(100, &rate, "EUR", "USD");
        assert!(matches!(result, Err(ConversionError::InvalidRate(_))));
    }

    #[test]
    fn convert_rejects_currency_mismatch() {
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(100, &rate, "GBP", "JPY");
        assert!(matches!(
            result,
            Err(ConversionError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn bankers_round_half_to_even() {
        // 0.5 → 0 (even)
        assert_eq!(bankers_round(0.5), 0);
        // 1.5 → 2 (even)
        assert_eq!(bankers_round(1.5), 2);
        // 2.5 → 2 (even)
        assert_eq!(bankers_round(2.5), 2);
        // 3.5 → 4 (even)
        assert_eq!(bankers_round(3.5), 4);
    }

    #[test]
    fn bankers_round_non_half() {
        assert_eq!(bankers_round(1.3), 1);
        assert_eq!(bankers_round(1.7), 2);
        assert_eq!(bankers_round(2.1), 2);
        assert_eq!(bankers_round(2.9), 3);
    }

    #[test]
    fn convert_balanced_journal() {
        // Balanced entry: DR 100.00 EUR, CR 100.00 EUR
        let rate = eur_usd_rate(1.085);
        let lines = vec![(10000, 0), (0, 10000)];
        let result = convert_journal_lines(&lines, &rate, "EUR", "USD").unwrap();

        assert_eq!(result.len(), 2);
        // Both should convert to 10850
        assert_eq!(result[0].rpt_debit_minor, 10850);
        assert_eq!(result[0].rpt_credit_minor, 0);
        assert_eq!(result[1].rpt_debit_minor, 0);
        assert_eq!(result[1].rpt_credit_minor, 10850);

        // Balance check
        let total_d: i64 = result.iter().map(|l| l.rpt_debit_minor).sum();
        let total_c: i64 = result.iter().map(|l| l.rpt_credit_minor).sum();
        assert_eq!(total_d, total_c, "Debits must equal credits");
    }

    #[test]
    fn convert_journal_with_rounding_residual() {
        // 3 lines that individually round differently:
        // DR 33.33 EUR = 3333 minor → 3333 * 1.085 = 3616.305 → 3616
        // DR 33.33 EUR = 3333 minor → 3616
        // CR 66.66 EUR = 6666 minor → 6666 * 1.085 = 7232.61 → 7233
        // Total DR = 7232, Total CR = 7233 → residual = -1
        // Adjustment: increase debit on largest line by 1
        let rate = eur_usd_rate(1.085);
        let lines = vec![(3333, 0), (3333, 0), (0, 6666)];
        let result = convert_journal_lines(&lines, &rate, "EUR", "USD").unwrap();

        let total_d: i64 = result.iter().map(|l| l.rpt_debit_minor).sum();
        let total_c: i64 = result.iter().map(|l| l.rpt_credit_minor).sum();
        assert_eq!(
            total_d, total_c,
            "Balance invariant must hold after adjustment"
        );
    }

    #[test]
    fn convert_preserves_original_amounts() {
        let rate = eur_usd_rate(1.085);
        let lines = vec![(10000, 0), (0, 10000)];
        let result = convert_journal_lines(&lines, &rate, "EUR", "USD").unwrap();

        assert_eq!(result[0].txn_debit_minor, 10000);
        assert_eq!(result[0].txn_credit_minor, 0);
        assert_eq!(result[1].txn_debit_minor, 0);
        assert_eq!(result[1].txn_credit_minor, 10000);
    }

    #[test]
    fn same_currency_no_conversion() {
        assert!(!requires_conversion("USD", "USD"));
        assert!(!requires_conversion("usd", "USD"));
    }

    #[test]
    fn different_currency_needs_conversion() {
        assert!(requires_conversion("EUR", "USD"));
        assert!(requires_conversion("GBP", "USD"));
    }

    #[test]
    fn resolve_rate_direct() {
        let rate = eur_usd_rate(1.085);
        let r = resolve_rate(&rate, "EUR", "USD").unwrap();
        assert!((r - 1.085).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_rate_inverse() {
        let rate = eur_usd_rate(1.085);
        let r = resolve_rate(&rate, "USD", "EUR").unwrap();
        assert!((r - rate.inverse_rate).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_rate_case_insensitive() {
        let rate = eur_usd_rate(1.085);
        let r = resolve_rate(&rate, "eur", "usd").unwrap();
        assert!((r - 1.085).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_rate_mismatch() {
        let rate = eur_usd_rate(1.085);
        let result = resolve_rate(&rate, "GBP", "JPY");
        assert!(matches!(
            result,
            Err(ConversionError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn convert_1_minor_unit() {
        // 0.01 EUR at 1.085 = 0.01085 → rounds to 1 minor
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(1, &rate, "EUR", "USD").unwrap();
        assert_eq!(result.reporting_amount_minor, 1);
    }

    #[test]
    fn convert_large_amount() {
        // 1,000,000.00 EUR = 100000000 minor at 1.085 = 108500000
        let rate = eur_usd_rate(1.085);
        let result = convert_amount(100_000_000, &rate, "EUR", "USD").unwrap();
        assert_eq!(result.reporting_amount_minor, 108_500_000);
    }

    #[test]
    fn convert_rate_near_parity() {
        // Rate very close to 1.0
        let rate = eur_usd_rate(1.0001);
        let result = convert_amount(100000, &rate, "EUR", "USD").unwrap();
        // 100000 * 1.0001 = 100010.0 → 100010
        assert_eq!(result.reporting_amount_minor, 100010);
    }

    #[test]
    fn convert_high_rate_jpy() {
        // USD/JPY at 150.25 → 100.00 USD = 15025 JPY = 1502500 minor (JPY uses no decimal)
        // Actually, for minor units, JPY: 1 JPY = 1 minor unit
        // But we treat all as 2-decimal minor units for consistency
        let rate = RateSnapshot {
            rate_id: Uuid::new_v4(),
            rate: 150.25,
            inverse_rate: 1.0 / 150.25,
            effective_at: Utc::now(),
            base_currency: "USD".to_string(),
            quote_currency: "JPY".to_string(),
        };
        let result = convert_amount(10000, &rate, "USD", "JPY").unwrap();
        // 10000 * 150.25 = 1502500
        assert_eq!(result.reporting_amount_minor, 1502500);
    }
}
