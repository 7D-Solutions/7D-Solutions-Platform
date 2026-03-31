//! Core types for probabilistic cash flow forecasting.

use serde::Serialize;

/// Empirical payment timing profile for a (tenant, customer, currency) tuple.
/// Built from `rpt_payment_history` days_to_pay values, sorted ascending.
#[derive(Debug, Clone)]
pub struct PaymentProfile {
    /// Sorted days-to-pay observations (ascending).
    pub observations: Vec<i32>,
    /// 25th percentile days-to-pay.
    pub p25: f64,
    /// 50th percentile (median) days-to-pay.
    pub p50: f64,
    /// 75th percentile days-to-pay.
    pub p75: f64,
}

/// Expected collection for a single forecast horizon.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ForecastHorizon {
    pub days: u32,
    pub expected_cents: i64,
    pub p25_cents: i64,
    pub p75_cents: i64,
}

/// An open invoice flagged as at-risk (P(30) < 0.40).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AtRiskItem {
    pub invoice_id: String,
    pub customer_id: String,
    pub currency: String,
    pub amount_cents: i64,
    pub p30: f64,
    pub age_days: i32,
}

/// Currency-grouped forecast result.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CurrencyForecast {
    pub currency: String,
    pub horizons: Vec<ForecastHorizon>,
    pub at_risk: Vec<AtRiskItem>,
}

/// Top-level forecast response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CashForecastResponse {
    pub as_of: chrono::DateTime<chrono::Utc>,
    pub results: Vec<CurrencyForecast>,
}
