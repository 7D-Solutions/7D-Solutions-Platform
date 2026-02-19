//! Pure straight-line depreciation computation. No I/O — fully deterministic.
//!
//! Monthly periods are anchored to the first calendar day of the in-service month.
//! The last period absorbs any integer-division remainder so the cumulative total
//! exactly equals the depreciable amount (cost − salvage).

use chrono::{Datelike, Duration, Months, NaiveDate};

/// One planned period in a straight-line depreciation schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodEntry {
    pub period_number: i32,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub depreciation_amount_minor: i64,
    pub cumulative_depreciation_minor: i64,
    pub remaining_book_value_minor: i64,
}

/// Compute a straight-line depreciation schedule.
///
/// - Periods are full calendar months starting from the first day of
///   `in_service_date`'s month.
/// - Monthly amount = `(cost − salvage) / useful_life_months` (integer division).
/// - The final period absorbs the remainder so cumulative == depreciable exactly.
/// - Returns an empty vec when depreciable amount == 0 or useful_life_months <= 0.
pub fn compute_straight_line(
    in_service_date: NaiveDate,
    acquisition_cost_minor: i64,
    salvage_value_minor: i64,
    useful_life_months: i32,
) -> Vec<PeriodEntry> {
    let depreciable = (acquisition_cost_minor - salvage_value_minor).max(0);
    if depreciable == 0 || useful_life_months <= 0 {
        return vec![];
    }

    let monthly = depreciable / useful_life_months as i64;
    let mut entries = Vec::with_capacity(useful_life_months as usize);
    let mut cumulative: i64 = 0;

    // Anchor to the first day of the in_service_date's month.
    let base = NaiveDate::from_ymd_opt(in_service_date.year(), in_service_date.month(), 1)
        .expect("in_service_date must be a valid date");

    for i in 0..useful_life_months {
        let period_start = base
            .checked_add_months(Months::new(i as u32))
            .expect("period_start overflow");
        let period_end = month_end(period_start);

        // Last period absorbs integer-division remainder.
        let amount = if i + 1 == useful_life_months {
            depreciable - cumulative
        } else {
            monthly
        };

        cumulative += amount;

        entries.push(PeriodEntry {
            period_number: i + 1,
            period_start,
            period_end,
            depreciation_amount_minor: amount,
            cumulative_depreciation_minor: cumulative,
            remaining_book_value_minor: acquisition_cost_minor - cumulative,
        });
    }

    entries
}

/// Last calendar day of the month containing `date`.
fn month_end(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
        .expect("valid month start")
        .checked_add_months(Months::new(1))
        .expect("next month overflow")
        - Duration::days(1)
}

// ============================================================================
// Unit tests — no database required
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn empty_when_zero_depreciable() {
        let entries = compute_straight_line(date(2026, 1, 1), 10_000, 10_000, 12);
        assert!(entries.is_empty(), "cost == salvage → no entries");
    }

    #[test]
    fn empty_when_zero_life() {
        let entries = compute_straight_line(date(2026, 1, 1), 10_000, 0, 0);
        assert!(entries.is_empty(), "zero life → no entries");
    }

    #[test]
    fn correct_period_count() {
        let entries = compute_straight_line(date(2026, 1, 15), 12_000, 0, 12);
        assert_eq!(entries.len(), 12);
    }

    #[test]
    fn cumulative_equals_depreciable() {
        // $1,200.00 cost, $0 salvage, 12 months → cumulative must equal 1_200_00
        let entries = compute_straight_line(date(2026, 1, 1), 120_000, 0, 12);
        let total: i64 = entries.iter().map(|e| e.depreciation_amount_minor).sum();
        assert_eq!(total, 120_000);
        let last = entries.last().unwrap();
        assert_eq!(last.cumulative_depreciation_minor, 120_000);
        assert_eq!(last.remaining_book_value_minor, 0);
    }

    #[test]
    fn salvage_respected() {
        // cost=100_000, salvage=10_000, 12 months → total depr = 90_000
        let entries = compute_straight_line(date(2026, 1, 1), 100_000, 10_000, 12);
        let total: i64 = entries.iter().map(|e| e.depreciation_amount_minor).sum();
        assert_eq!(total, 90_000);
        let last = entries.last().unwrap();
        // NBV at end = salvage value
        assert_eq!(last.remaining_book_value_minor, 10_000);
    }

    #[test]
    fn remainder_absorbed_in_last_period() {
        // 10_001 / 3 = 3_333 r2 → periods [3333, 3333, 3335]
        let entries = compute_straight_line(date(2026, 1, 1), 10_001, 0, 3);
        assert_eq!(entries[0].depreciation_amount_minor, 3_333);
        assert_eq!(entries[1].depreciation_amount_minor, 3_333);
        assert_eq!(entries[2].depreciation_amount_minor, 3_335);
        let total: i64 = entries.iter().map(|e| e.depreciation_amount_minor).sum();
        assert_eq!(total, 10_001);
    }

    #[test]
    fn period_numbers_sequential() {
        let entries = compute_straight_line(date(2026, 3, 10), 60_000, 0, 6);
        for (i, e) in entries.iter().enumerate() {
            assert_eq!(e.period_number, i as i32 + 1);
        }
    }

    #[test]
    fn period_dates_anchored_to_month_start() {
        let entries = compute_straight_line(date(2026, 1, 15), 12_000, 0, 3);
        // Period 1 starts 2026-01-01 regardless of in_service day
        assert_eq!(entries[0].period_start, date(2026, 1, 1));
        assert_eq!(entries[0].period_end, date(2026, 1, 31));
        assert_eq!(entries[1].period_start, date(2026, 2, 1));
        assert_eq!(entries[1].period_end, date(2026, 2, 28));
        assert_eq!(entries[2].period_start, date(2026, 3, 1));
        assert_eq!(entries[2].period_end, date(2026, 3, 31));
    }

    #[test]
    fn period_end_leap_year() {
        // Feb 2024 is a leap year
        let entries = compute_straight_line(date(2024, 2, 1), 2_400, 0, 1);
        assert_eq!(entries[0].period_end, date(2024, 2, 29));
    }

    #[test]
    fn year_crossing() {
        let entries = compute_straight_line(date(2026, 11, 1), 24_000, 0, 3);
        assert_eq!(entries[0].period_start, date(2026, 11, 1));
        assert_eq!(entries[1].period_start, date(2026, 12, 1));
        assert_eq!(entries[2].period_start, date(2027, 1, 1));
        assert_eq!(entries[2].period_end, date(2027, 1, 31));
    }
}
