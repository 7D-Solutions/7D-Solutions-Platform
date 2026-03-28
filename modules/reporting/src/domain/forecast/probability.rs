//! Conditional probability computation for cash flow forecasting.
//!
//! Core formula:
//!   P(pay in next N days | unpaid at age A) = (F(A+N) - F(A)) / (1 - F(A))
//!   where F(x) = empirical CDF at x days
//!
//! Handles edge cases:
//!   - F(A) == 1.0 → return 0.0 (already past all observed payments)
//!   - Result clamped to [0.0, 1.0]

use super::types::PaymentProfile;

/// Compute the empirical CDF value F(x): fraction of observations <= x.
///
/// Uses binary search (O(log n)) since observations are sorted ascending.
pub fn empirical_cdf(profile: &PaymentProfile, x: f64) -> f64 {
    if profile.observations.is_empty() {
        return 0.0;
    }
    let x_floor = x.floor() as i32;
    // partition_point returns the index of the first element > x_floor,
    // which equals the count of elements <= x_floor (since obs are sorted).
    let count = profile.observations.partition_point(|&d| d <= x_floor);
    count as f64 / profile.observations.len() as f64
}

/// Compute P(pay within next N days | invoice is already A days old).
///
/// Returns a probability in [0.0, 1.0].
/// If F(A) == 1.0, returns 0.0 (all historical invoices were already paid
/// by this age, so this one is an outlier with no basis for estimation).
pub fn compute_conditional_p(profile: &PaymentProfile, age_days: u32, horizon_days: u32) -> f64 {
    let fa = empirical_cdf(profile, age_days as f64);
    if fa >= 1.0 {
        return 0.0;
    }
    let fa_n = empirical_cdf(profile, (age_days + horizon_days) as f64);
    let p = (fa_n - fa) / (1.0 - fa);
    p.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> PaymentProfile {
        // 5 observations: 10, 20, 30, 40, 50 days
        PaymentProfile {
            observations: vec![10, 20, 30, 40, 50],
            p25: 20.0,
            p50: 30.0,
            p75: 40.0,
        }
    }

    #[test]
    fn test_empirical_cdf_at_boundaries() {
        let p = sample_profile();
        assert!((empirical_cdf(&p, 5.0) - 0.0).abs() < 0.001);
        assert!((empirical_cdf(&p, 10.0) - 0.2).abs() < 0.001);
        assert!((empirical_cdf(&p, 25.0) - 0.4).abs() < 0.001);
        assert!((empirical_cdf(&p, 50.0) - 1.0).abs() < 0.001);
        assert!((empirical_cdf(&p, 100.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_conditional_p_fresh_invoice() {
        let p = sample_profile();
        // Age 0, horizon 30 → F(30)/1.0 = 3/5 = 0.6
        let prob = compute_conditional_p(&p, 0, 30);
        assert!((prob - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_conditional_p_aged_invoice() {
        let p = sample_profile();
        // Age 20, horizon 30 → F(50) - F(20) / (1 - F(20))
        // F(50) = 1.0, F(20) = 0.4 → (1.0 - 0.4) / (1 - 0.4) = 1.0
        let prob = compute_conditional_p(&p, 20, 30);
        assert!((prob - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_conditional_p_all_past() {
        let p = sample_profile();
        // Age 60 → F(60) = 1.0 → returns 0.0
        let prob = compute_conditional_p(&p, 60, 30);
        assert!((prob - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_conditional_p_partial() {
        let p = sample_profile();
        // Age 10, horizon 20 → F(30) - F(10) / (1 - F(10))
        // F(30) = 3/5 = 0.6, F(10) = 1/5 = 0.2
        // (0.6 - 0.2) / (1 - 0.2) = 0.4 / 0.8 = 0.5
        let prob = compute_conditional_p(&p, 10, 20);
        assert!((prob - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_empty_profile() {
        let p = PaymentProfile {
            observations: vec![],
            p25: 0.0,
            p50: 0.0,
            p75: 0.0,
        };
        let prob = compute_conditional_p(&p, 0, 30);
        assert!((prob - 0.0).abs() < 0.001);
    }
}
