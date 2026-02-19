//! Forecast assumptions — declares rates and methodology used by the cash forecast.
//!
//! The forecast is advisory. These default assumptions encode conservative
//! collection and payment expectations based on aging bucket age. Callers
//! can override via query parameters in the future.

use serde::Serialize;

/// Assumed collection/payment rates per aging bucket.
///
/// Rates are expressed as fractions (0.0–1.0). A rate of 0.95 means
/// 95% of the bucket value is expected to convert to a cash flow.
#[derive(Debug, Clone, Serialize)]
pub struct ForecastAssumptions {
    /// Expected collection rate for AR "current" (not yet due).
    pub ar_current_rate: f64,
    /// Expected collection rate for AR 1–30 days overdue.
    pub ar_1_30_rate: f64,
    /// Expected collection rate for AR 31–60 days overdue.
    pub ar_31_60_rate: f64,
    /// Expected collection rate for AR 61–90 days overdue.
    pub ar_61_90_rate: f64,
    /// Expected collection rate for AR over 90 days overdue.
    pub ar_over_90_rate: f64,
    /// Expected payment rate for AP "current" (not yet due).
    pub ap_current_rate: f64,
    /// Expected payment rate for AP 1–30 days overdue.
    pub ap_1_30_rate: f64,
    /// Expected payment rate for AP 31–60 days overdue.
    pub ap_31_60_rate: f64,
    /// Expected payment rate for AP 61–90 days overdue.
    pub ap_61_90_rate: f64,
    /// Expected payment rate for AP over 90 days overdue.
    pub ap_over_90_rate: f64,
    /// Rate applied to scheduled (pending) payment runs.
    pub scheduled_payment_rate: f64,
}

impl Default for ForecastAssumptions {
    fn default() -> Self {
        Self {
            // AR: collection likelihood decreases with age
            ar_current_rate: 0.95,
            ar_1_30_rate: 0.85,
            ar_31_60_rate: 0.70,
            ar_61_90_rate: 0.50,
            ar_over_90_rate: 0.25,
            // AP: all approved payables are expected to be paid
            ap_current_rate: 1.0,
            ap_1_30_rate: 1.0,
            ap_31_60_rate: 1.0,
            ap_61_90_rate: 1.0,
            ap_over_90_rate: 1.0,
            // Scheduled payment runs: high confidence (already approved)
            scheduled_payment_rate: 1.0,
        }
    }
}

impl ForecastAssumptions {
    /// Human-readable description of the methodology.
    pub fn methodology_note() -> &'static str {
        "Deterministic forecast based on AR aging (expected inflows) and AP aging \
         (expected outflows). AR collection rates decrease with bucket age \
         (95% current → 25% over 90 days). AP payments assumed at 100% for \
         all buckets. Scheduled payment runs included at 100%. \
         All amounts in minor currency units. Forecast is advisory only."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_assumptions_are_reasonable() {
        let a = ForecastAssumptions::default();
        // AR rates should decrease with age
        assert!(a.ar_current_rate > a.ar_1_30_rate);
        assert!(a.ar_1_30_rate > a.ar_31_60_rate);
        assert!(a.ar_31_60_rate > a.ar_61_90_rate);
        assert!(a.ar_61_90_rate > a.ar_over_90_rate);
        // AP rates should all be 1.0
        assert_eq!(a.ap_current_rate, 1.0);
        assert_eq!(a.ap_over_90_rate, 1.0);
        // All rates in valid range
        for rate in [
            a.ar_current_rate,
            a.ar_1_30_rate,
            a.ar_31_60_rate,
            a.ar_61_90_rate,
            a.ar_over_90_rate,
            a.ap_current_rate,
            a.ap_1_30_rate,
            a.ap_31_60_rate,
            a.ap_61_90_rate,
            a.ap_over_90_rate,
            a.scheduled_payment_rate,
        ] {
            assert!((0.0..=1.0).contains(&rate), "rate {} out of range", rate);
        }
    }

    #[test]
    fn methodology_note_is_non_empty() {
        assert!(!ForecastAssumptions::methodology_note().is_empty());
    }
}
