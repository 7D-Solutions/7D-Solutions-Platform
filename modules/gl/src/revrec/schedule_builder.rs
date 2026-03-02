//! Schedule Builder — deterministic recognition schedule generation (Phase 24a)
//!
//! Generates amortization schedules for performance obligations based on their
//! recognition pattern. V1 baseline supports:
//! - RatableOverTime: straight-line allocation across monthly periods
//! - PointInTime: single-period full recognition
//!
//! ## Determinism guarantee
//! Same inputs (obligation_id, allocated_amount, pattern, satisfaction dates)
//! always produce identical schedule lines. No randomness, no clock dependency.
//!
//! ## Rounding
//! Straight-line division distributes the remainder penny-by-penny across the
//! earliest periods (largest-remainder method). This ensures:
//! - sum(lines) == total_allocated (exact, no rounding loss)
//! - lines differ by at most 1 minor unit

use chrono::{Datelike, NaiveDate};
use uuid::Uuid;

use super::{PerformanceObligation, RecognitionPattern, ScheduleCreatedPayload, ScheduleLine};

/// Errors from schedule generation
#[derive(Debug, thiserror::Error)]
pub enum ScheduleBuildError {
    #[error("Obligation has zero or negative allocation: {0}")]
    InvalidAllocation(i64),

    #[error("Invalid satisfaction_start date: {0}")]
    InvalidStartDate(String),

    #[error("Invalid satisfaction_end date: {0}")]
    InvalidEndDate(String),

    #[error("satisfaction_end {end} is before satisfaction_start {start}")]
    EndBeforeStart { start: String, end: String },

    #[error("RatableOverTime requires period_months >= 1, got {0}")]
    InvalidPeriodMonths(u32),

    #[error("UsageBased recognition is not supported in schedule builder v1")]
    UsageBasedNotSupported,
}

/// Generate a recognition schedule for a performance obligation.
///
/// The schedule is deterministic: identical inputs always produce identical output.
/// The schedule_id is caller-supplied for idempotency.
///
/// Returns a `ScheduleCreatedPayload` ready for persistence and outbox emission.
pub fn generate_schedule(
    schedule_id: Uuid,
    contract_id: Uuid,
    obligation: &PerformanceObligation,
    tenant_id: &str,
    currency: &str,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<ScheduleCreatedPayload, ScheduleBuildError> {
    if obligation.allocated_amount_minor <= 0 {
        return Err(ScheduleBuildError::InvalidAllocation(
            obligation.allocated_amount_minor,
        ));
    }

    let lines = match &obligation.recognition_pattern {
        RecognitionPattern::RatableOverTime { period_months } => {
            generate_ratable_lines(obligation, *period_months)?
        }
        RecognitionPattern::PointInTime => generate_point_in_time_lines(obligation)?,
        RecognitionPattern::UsageBased { .. } => {
            return Err(ScheduleBuildError::UsageBasedNotSupported);
        }
    };

    let first_period = lines.first().map(|l| l.period.clone()).unwrap_or_default();
    let last_period = lines.last().map(|l| l.period.clone()).unwrap_or_default();

    Ok(ScheduleCreatedPayload {
        schedule_id,
        contract_id,
        obligation_id: obligation.obligation_id,
        tenant_id: tenant_id.to_string(),
        total_to_recognize_minor: obligation.allocated_amount_minor,
        currency: currency.to_string(),
        lines,
        first_period,
        last_period,
        created_at,
    })
}

/// Generate straight-line (ratable) schedule lines.
///
/// Divides the total allocation evenly across `period_months` consecutive months
/// starting from `satisfaction_start`. Uses the largest-remainder method to
/// distribute any remainder penny-by-penny to the earliest periods.
fn generate_ratable_lines(
    obligation: &PerformanceObligation,
    period_months: u32,
) -> Result<Vec<ScheduleLine>, ScheduleBuildError> {
    if period_months == 0 {
        return Err(ScheduleBuildError::InvalidPeriodMonths(period_months));
    }

    let start_date = NaiveDate::parse_from_str(&obligation.satisfaction_start, "%Y-%m-%d")
        .map_err(|_| ScheduleBuildError::InvalidStartDate(obligation.satisfaction_start.clone()))?;

    let total = obligation.allocated_amount_minor;
    let n = period_months as i64;
    let base_amount = total / n;
    let remainder = total % n;

    let mut lines = Vec::with_capacity(period_months as usize);
    let mut current_year = start_date.year();
    let mut current_month = start_date.month();

    for i in 0..period_months {
        // Distribute remainder to earliest periods (largest-remainder method)
        let amount = if (i as i64) < remainder {
            base_amount + 1
        } else {
            base_amount
        };

        let period = format!("{:04}-{:02}", current_year, current_month);

        lines.push(ScheduleLine {
            period,
            amount_to_recognize_minor: amount,
            deferred_revenue_account: "DEFERRED_REV".to_string(),
            recognized_revenue_account: "REV".to_string(),
        });

        // Advance to next month
        if current_month == 12 {
            current_month = 1;
            current_year += 1;
        } else {
            current_month += 1;
        }
    }

    Ok(lines)
}

/// Generate a single-period schedule line for point-in-time recognition.
///
/// The full allocation is recognized in the satisfaction_start period.
fn generate_point_in_time_lines(
    obligation: &PerformanceObligation,
) -> Result<Vec<ScheduleLine>, ScheduleBuildError> {
    let start_date = NaiveDate::parse_from_str(&obligation.satisfaction_start, "%Y-%m-%d")
        .map_err(|_| ScheduleBuildError::InvalidStartDate(obligation.satisfaction_start.clone()))?;

    let period = format!("{:04}-{:02}", start_date.year(), start_date.month());

    Ok(vec![ScheduleLine {
        period,
        amount_to_recognize_minor: obligation.allocated_amount_minor,
        deferred_revenue_account: "DEFERRED_REV".to_string(),
        recognized_revenue_account: "REV".to_string(),
    }])
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn ratable_obligation(amount: i64, months: u32) -> PerformanceObligation {
        PerformanceObligation {
            obligation_id: Uuid::new_v4(),
            name: "SaaS License".to_string(),
            description: "Platform access".to_string(),
            allocated_amount_minor: amount,
            recognition_pattern: RecognitionPattern::RatableOverTime {
                period_months: months,
            },
            satisfaction_start: "2026-01-01".to_string(),
            satisfaction_end: Some("2026-12-31".to_string()),
        }
    }

    fn point_in_time_obligation(amount: i64) -> PerformanceObligation {
        PerformanceObligation {
            obligation_id: Uuid::new_v4(),
            name: "Implementation".to_string(),
            description: "One-time setup".to_string(),
            allocated_amount_minor: amount,
            recognition_pattern: RecognitionPattern::PointInTime,
            satisfaction_start: "2026-03-15".to_string(),
            satisfaction_end: None,
        }
    }

    #[test]
    fn ratable_12_months_even_division() {
        let obligation = ratable_obligation(120000_00, 12);
        let payload = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        )
        .unwrap();

        assert_eq!(payload.lines.len(), 12);
        assert_eq!(payload.first_period, "2026-01");
        assert_eq!(payload.last_period, "2026-12");

        // All lines should be $10,000.00
        for line in &payload.lines {
            assert_eq!(line.amount_to_recognize_minor, 10000_00);
        }

        // Sum must equal total
        let sum: i64 = payload
            .lines
            .iter()
            .map(|l| l.amount_to_recognize_minor)
            .sum();
        assert_eq!(sum, 120000_00);
    }

    #[test]
    fn ratable_with_remainder_distributes_pennies() {
        // $100.00 over 3 months = $33.33, $33.33, $33.34? No — $33.34, $33.33, $33.33
        // Actually: 10000 / 3 = 3333 remainder 1 → first period gets +1
        let obligation = ratable_obligation(10000, 3);
        let payload = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        )
        .unwrap();

        assert_eq!(payload.lines.len(), 3);
        assert_eq!(payload.lines[0].amount_to_recognize_minor, 3334); // gets extra penny
        assert_eq!(payload.lines[1].amount_to_recognize_minor, 3333);
        assert_eq!(payload.lines[2].amount_to_recognize_minor, 3333);

        let sum: i64 = payload
            .lines
            .iter()
            .map(|l| l.amount_to_recognize_minor)
            .sum();
        assert_eq!(sum, 10000);
    }

    #[test]
    fn ratable_7_months_remainder_5() {
        // $100.00 over 7 months: 10000 / 7 = 1428 remainder 4
        // First 4 get 1429, last 3 get 1428
        let obligation = ratable_obligation(10000, 7);
        let payload = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        )
        .unwrap();

        assert_eq!(payload.lines.len(), 7);
        for i in 0..4 {
            assert_eq!(payload.lines[i].amount_to_recognize_minor, 1429);
        }
        for i in 4..7 {
            assert_eq!(payload.lines[i].amount_to_recognize_minor, 1428);
        }

        let sum: i64 = payload
            .lines
            .iter()
            .map(|l| l.amount_to_recognize_minor)
            .sum();
        assert_eq!(sum, 10000);
    }

    #[test]
    fn point_in_time_single_line() {
        let obligation = point_in_time_obligation(24000_00);
        let payload = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        )
        .unwrap();

        assert_eq!(payload.lines.len(), 1);
        assert_eq!(payload.first_period, "2026-03");
        assert_eq!(payload.last_period, "2026-03");
        assert_eq!(payload.lines[0].amount_to_recognize_minor, 24000_00);
        assert_eq!(payload.lines[0].period, "2026-03");
    }

    #[test]
    fn determinism_same_inputs_same_output() {
        let schedule_id = Uuid::new_v4();
        let contract_id = Uuid::new_v4();
        let obligation_id = Uuid::new_v4();
        let created_at = Utc::now();

        let obligation = PerformanceObligation {
            obligation_id,
            name: "License".to_string(),
            description: "Access".to_string(),
            allocated_amount_minor: 120000_00,
            recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
            satisfaction_start: "2026-01-01".to_string(),
            satisfaction_end: Some("2026-12-31".to_string()),
        };

        let result1 = generate_schedule(
            schedule_id,
            contract_id,
            &obligation,
            "tenant-1",
            "USD",
            created_at,
        )
        .unwrap();

        let result2 = generate_schedule(
            schedule_id,
            contract_id,
            &obligation,
            "tenant-1",
            "USD",
            created_at,
        )
        .unwrap();

        assert_eq!(result1.lines.len(), result2.lines.len());
        for (a, b) in result1.lines.iter().zip(result2.lines.iter()) {
            assert_eq!(a.period, b.period);
            assert_eq!(a.amount_to_recognize_minor, b.amount_to_recognize_minor);
        }
    }

    #[test]
    fn year_boundary_crossing() {
        // Start in November, 4 months → Nov, Dec, Jan, Feb
        let obligation = PerformanceObligation {
            obligation_id: Uuid::new_v4(),
            name: "Q4 Bridge".to_string(),
            description: "Cross-year service".to_string(),
            allocated_amount_minor: 40000_00,
            recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 4 },
            satisfaction_start: "2026-11-01".to_string(),
            satisfaction_end: Some("2027-02-28".to_string()),
        };

        let payload = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        )
        .unwrap();

        assert_eq!(payload.lines.len(), 4);
        assert_eq!(payload.lines[0].period, "2026-11");
        assert_eq!(payload.lines[1].period, "2026-12");
        assert_eq!(payload.lines[2].period, "2027-01");
        assert_eq!(payload.lines[3].period, "2027-02");
    }

    #[test]
    fn usage_based_returns_error() {
        let obligation = PerformanceObligation {
            obligation_id: Uuid::new_v4(),
            name: "API Usage".to_string(),
            description: "Usage commitment".to_string(),
            allocated_amount_minor: 50000_00,
            recognition_pattern: RecognitionPattern::UsageBased {
                metric: "api_calls".to_string(),
                total_contracted_quantity: 1_000_000.0,
                unit: "calls".to_string(),
            },
            satisfaction_start: "2026-01-01".to_string(),
            satisfaction_end: Some("2026-12-31".to_string()),
        };

        let result = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ScheduleBuildError::UsageBasedNotSupported
        ));
    }

    #[test]
    fn zero_allocation_rejected() {
        let obligation = ratable_obligation(0, 12);
        let result = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        );
        assert!(matches!(
            result.unwrap_err(),
            ScheduleBuildError::InvalidAllocation(0)
        ));
    }

    #[test]
    fn zero_period_months_rejected() {
        let obligation = PerformanceObligation {
            obligation_id: Uuid::new_v4(),
            name: "Bad".to_string(),
            description: "Zero months".to_string(),
            allocated_amount_minor: 10000,
            recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 0 },
            satisfaction_start: "2026-01-01".to_string(),
            satisfaction_end: None,
        };
        let result = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        );
        assert!(matches!(
            result.unwrap_err(),
            ScheduleBuildError::InvalidPeriodMonths(0)
        ));
    }

    #[test]
    fn accounts_default_to_standard_names() {
        let obligation = ratable_obligation(12000, 3);
        let payload = generate_schedule(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &obligation,
            "tenant-1",
            "USD",
            Utc::now(),
        )
        .unwrap();

        for line in &payload.lines {
            assert_eq!(line.deferred_revenue_account, "DEFERRED_REV");
            assert_eq!(line.recognized_revenue_account, "REV");
        }
    }
}
