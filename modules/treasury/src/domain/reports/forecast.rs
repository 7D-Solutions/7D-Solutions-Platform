//! Cash forecast v1 — deterministic projection from AR/AP aging data.
//!
//! Computes expected inflows (from AR receivables) and outflows (from AP
//! payables + scheduled payment runs), grouped by currency and time bucket.
//!
//! Design:
//! - Pure computation: `compute_forecast` takes typed inputs and assumptions,
//!   returns a forecast response. No side effects.
//! - Cross-module reads: helper functions query AR/AP databases (read-only)
//!   to obtain aging data. Treasury never writes to those databases.
//! - Assumptions are explicit and included in the response so the caller
//!   can evaluate the forecast's basis.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::BTreeMap;

use super::assumptions::ForecastAssumptions;

// ============================================================================
// Input types (mirrors of AR/AP aging structures)
// ============================================================================

/// AR aging summary for a single currency.
#[derive(Debug, Clone, Default)]
pub struct ArAgingInput {
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub days_over_90_minor: i64,
}

/// AP aging summary for a single currency.
#[derive(Debug, Clone, Default)]
pub struct ApAgingInput {
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub over_90_minor: i64,
}

/// Scheduled payment run (pending, not yet executed).
#[derive(Debug, Clone)]
pub struct ScheduledPaymentInput {
    pub currency: String,
    pub total_minor: i64,
}

// ============================================================================
// Output types
// ============================================================================

/// Forecast for a single currency.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CurrencyForecast {
    pub currency: String,
    pub inflows: ForecastBuckets,
    pub outflows: ForecastBuckets,
    pub scheduled_outflows_minor: i64,
    pub net_by_bucket: ForecastBuckets,
    pub total_net_minor: i64,
}

/// Amounts per aging time bucket, after applying assumption rates.
#[derive(Debug, Clone, Serialize, Default, utoipa::ToSchema)]
pub struct ForecastBuckets {
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub over_90_minor: i64,
    pub total_minor: i64,
}

/// Full forecast response.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ForecastResponse {
    pub as_of: DateTime<Utc>,
    pub forecasts: Vec<CurrencyForecast>,
    pub assumptions: ForecastAssumptions,
    pub methodology: String,
    pub data_sources: Vec<String>,
}

// ============================================================================
// Computation
// ============================================================================

/// Compute a cash forecast from AR/AP aging inputs and scheduled payments.
///
/// This is a pure function — all data must be provided as arguments.
/// The forecast applies assumption rates to raw aging amounts to produce
/// expected cash flows per time bucket and currency.
pub fn compute_forecast(
    ar_aging: &[ArAgingInput],
    ap_aging: &[ApAgingInput],
    scheduled_payments: &[ScheduledPaymentInput],
    assumptions: &ForecastAssumptions,
    data_sources: Vec<String>,
) -> ForecastResponse {
    // Collect all currencies
    let mut currencies: BTreeMap<String, (Option<&ArAgingInput>, Option<&ApAgingInput>, i64)> =
        BTreeMap::new();

    for ar in ar_aging {
        currencies
            .entry(ar.currency.clone())
            .or_insert((None, None, 0))
            .0 = Some(ar);
    }
    for ap in ap_aging {
        currencies
            .entry(ap.currency.clone())
            .or_insert((None, None, 0))
            .1 = Some(ap);
    }
    for sp in scheduled_payments {
        currencies
            .entry(sp.currency.clone())
            .or_insert((None, None, 0))
            .2 += sp.total_minor;
    }

    let forecasts: Vec<CurrencyForecast> = currencies
        .into_iter()
        .map(|(currency, (ar, ap, sched))| {
            let inflows = match ar {
                Some(ar) => ForecastBuckets {
                    current_minor: apply_rate(ar.current_minor, assumptions.ar_current_rate),
                    days_1_30_minor: apply_rate(ar.days_1_30_minor, assumptions.ar_1_30_rate),
                    days_31_60_minor: apply_rate(ar.days_31_60_minor, assumptions.ar_31_60_rate),
                    days_61_90_minor: apply_rate(ar.days_61_90_minor, assumptions.ar_61_90_rate),
                    over_90_minor: apply_rate(ar.days_over_90_minor, assumptions.ar_over_90_rate),
                    total_minor: 0, // computed below
                },
                None => ForecastBuckets::default(),
            };
            let inflows = ForecastBuckets {
                total_minor: inflows.current_minor
                    + inflows.days_1_30_minor
                    + inflows.days_31_60_minor
                    + inflows.days_61_90_minor
                    + inflows.over_90_minor,
                ..inflows
            };

            let outflows = match ap {
                Some(ap) => ForecastBuckets {
                    current_minor: apply_rate(ap.current_minor, assumptions.ap_current_rate),
                    days_1_30_minor: apply_rate(ap.days_1_30_minor, assumptions.ap_1_30_rate),
                    days_31_60_minor: apply_rate(ap.days_31_60_minor, assumptions.ap_31_60_rate),
                    days_61_90_minor: apply_rate(ap.days_61_90_minor, assumptions.ap_61_90_rate),
                    over_90_minor: apply_rate(ap.over_90_minor, assumptions.ap_over_90_rate),
                    total_minor: 0,
                },
                None => ForecastBuckets::default(),
            };
            let outflows = ForecastBuckets {
                total_minor: outflows.current_minor
                    + outflows.days_1_30_minor
                    + outflows.days_31_60_minor
                    + outflows.days_61_90_minor
                    + outflows.over_90_minor,
                ..outflows
            };

            let sched_out = apply_rate(sched, assumptions.scheduled_payment_rate);

            let net = ForecastBuckets {
                current_minor: inflows.current_minor - outflows.current_minor,
                days_1_30_minor: inflows.days_1_30_minor - outflows.days_1_30_minor,
                days_31_60_minor: inflows.days_31_60_minor - outflows.days_31_60_minor,
                days_61_90_minor: inflows.days_61_90_minor - outflows.days_61_90_minor,
                over_90_minor: inflows.over_90_minor - outflows.over_90_minor,
                total_minor: inflows.total_minor - outflows.total_minor - sched_out,
            };

            let total_net = inflows.total_minor - outflows.total_minor - sched_out;

            CurrencyForecast {
                currency,
                inflows,
                outflows,
                scheduled_outflows_minor: sched_out,
                net_by_bucket: net,
                total_net_minor: total_net,
            }
        })
        .collect();

    ForecastResponse {
        as_of: Utc::now(),
        forecasts,
        assumptions: assumptions.clone(),
        methodology: ForecastAssumptions::methodology_note().to_string(),
        data_sources,
    }
}

/// Apply a rate (0.0–1.0) to a minor-unit amount, rounding to nearest integer.
fn apply_rate(amount: i64, rate: f64) -> i64 {
    (amount as f64 * rate).round() as i64
}

// ============================================================================
// Cross-module read queries
// ============================================================================

/// Read AR aging summary grouped by currency from the AR database.
///
/// Queries the `ar_aging_buckets` projection table. This is a read-only
/// cross-module query — treasury never writes to the AR database.
pub async fn read_ar_aging(
    ar_pool: &PgPool,
    app_id: &str,
) -> Result<Vec<ArAgingInput>, sqlx::Error> {
    let rows: Vec<ArAgingRow> = sqlx::query_as(
        r#"
        SELECT
            currency,
            COALESCE(SUM(current_minor), 0)::bigint       AS current_minor,
            COALESCE(SUM(days_1_30_minor), 0)::bigint      AS days_1_30_minor,
            COALESCE(SUM(days_31_60_minor), 0)::bigint     AS days_31_60_minor,
            COALESCE(SUM(days_61_90_minor), 0)::bigint     AS days_61_90_minor,
            COALESCE(SUM(days_over_90_minor), 0)::bigint   AS days_over_90_minor
        FROM ar_aging_buckets
        WHERE app_id = $1
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(app_id)
    .fetch_all(ar_pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ArAgingInput {
            currency: r.currency,
            current_minor: r.current_minor,
            days_1_30_minor: r.days_1_30_minor,
            days_31_60_minor: r.days_31_60_minor,
            days_61_90_minor: r.days_61_90_minor,
            days_over_90_minor: r.days_over_90_minor,
        })
        .collect())
}

#[derive(Debug, sqlx::FromRow)]
struct ArAgingRow {
    currency: String,
    current_minor: i64,
    days_1_30_minor: i64,
    days_31_60_minor: i64,
    days_61_90_minor: i64,
    days_over_90_minor: i64,
}

/// Read AP aging summary grouped by currency from the AP database.
///
/// Computes open balances from `vendor_bills` minus `ap_allocations`,
/// then buckets by days-past-due. Read-only cross-module query.
pub async fn read_ap_aging(
    ap_pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<ApAgingInput>, sqlx::Error> {
    let rows: Vec<ApAgingRow> = sqlx::query_as(
        r#"
        WITH bill_open AS (
            SELECT
                b.currency,
                b.due_date,
                (b.total_minor - COALESCE(SUM(a.amount_minor), 0)) AS open_minor
            FROM vendor_bills b
            LEFT JOIN ap_allocations a
                ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
            WHERE b.tenant_id = $1
              AND b.status IN ('approved', 'partially_paid')
            GROUP BY b.bill_id, b.currency, b.due_date, b.total_minor
            HAVING (b.total_minor - COALESCE(SUM(a.amount_minor), 0)) > 0
        )
        SELECT
            currency,
            COALESCE(SUM(CASE WHEN due_date >= NOW()
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS current_minor,
            COALESCE(SUM(CASE WHEN due_date >= NOW() - INTERVAL '30 days'
                               AND due_date < NOW()
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN due_date >= NOW() - INTERVAL '60 days'
                               AND due_date < NOW() - INTERVAL '30 days'
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN due_date >= NOW() - INTERVAL '90 days'
                               AND due_date < NOW() - INTERVAL '60 days'
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN due_date < NOW() - INTERVAL '90 days'
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS over_90_minor
        FROM bill_open
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .fetch_all(ap_pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ApAgingInput {
            currency: r.currency,
            current_minor: r.current_minor,
            days_1_30_minor: r.days_1_30_minor,
            days_31_60_minor: r.days_31_60_minor,
            days_61_90_minor: r.days_61_90_minor,
            over_90_minor: r.over_90_minor,
        })
        .collect())
}

#[derive(Debug, sqlx::FromRow)]
struct ApAgingRow {
    currency: String,
    current_minor: i64,
    days_1_30_minor: i64,
    days_31_60_minor: i64,
    days_61_90_minor: i64,
    over_90_minor: i64,
}

/// Read pending (not yet executed) payment runs from the AP database.
pub async fn read_scheduled_payments(
    ap_pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<ScheduledPaymentInput>, sqlx::Error> {
    let rows: Vec<SchedRow> = sqlx::query_as(
        r#"
        SELECT
            currency,
            COALESCE(SUM(total_minor), 0)::bigint AS total_minor
        FROM payment_runs
        WHERE tenant_id = $1
          AND status = 'pending'
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .fetch_all(ap_pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScheduledPaymentInput {
            currency: r.currency,
            total_minor: r.total_minor,
        })
        .collect())
}

#[derive(Debug, sqlx::FromRow)]
struct SchedRow {
    currency: String,
    total_minor: i64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> ForecastAssumptions {
        ForecastAssumptions::default()
    }

    #[test]
    fn empty_inputs_produce_empty_forecast() {
        let resp = compute_forecast(&[], &[], &[], &defaults(), vec![]);
        assert!(resp.forecasts.is_empty());
        assert!(!resp.methodology.is_empty());
    }

    #[test]
    fn ar_only_produces_inflows() {
        let ar = vec![ArAgingInput {
            currency: "USD".to_string(),
            current_minor: 100_000,
            days_1_30_minor: 50_000,
            days_31_60_minor: 0,
            days_61_90_minor: 0,
            days_over_90_minor: 0,
        }];
        let resp = compute_forecast(&ar, &[], &[], &defaults(), vec![]);
        assert_eq!(resp.forecasts.len(), 1);
        let f = &resp.forecasts[0];
        assert_eq!(f.currency, "USD");
        // current: 100_000 * 0.95 = 95_000
        assert_eq!(f.inflows.current_minor, 95_000);
        // 1-30: 50_000 * 0.85 = 42_500
        assert_eq!(f.inflows.days_1_30_minor, 42_500);
        assert_eq!(f.inflows.total_minor, 95_000 + 42_500);
        // No outflows
        assert_eq!(f.outflows.total_minor, 0);
        assert_eq!(f.total_net_minor, 95_000 + 42_500);
    }

    #[test]
    fn ap_only_produces_outflows() {
        let ap = vec![ApAgingInput {
            currency: "USD".to_string(),
            current_minor: 80_000,
            days_1_30_minor: 20_000,
            days_31_60_minor: 0,
            days_61_90_minor: 0,
            over_90_minor: 0,
        }];
        let resp = compute_forecast(&[], &ap, &[], &defaults(), vec![]);
        assert_eq!(resp.forecasts.len(), 1);
        let f = &resp.forecasts[0];
        // AP rates are all 1.0, so outflows = raw amounts
        assert_eq!(f.outflows.current_minor, 80_000);
        assert_eq!(f.outflows.days_1_30_minor, 20_000);
        assert_eq!(f.outflows.total_minor, 100_000);
        assert_eq!(f.inflows.total_minor, 0);
        assert_eq!(f.total_net_minor, -100_000);
    }

    #[test]
    fn mixed_ar_ap_net_calculation() {
        let ar = vec![ArAgingInput {
            currency: "USD".to_string(),
            current_minor: 200_000,
            days_1_30_minor: 0,
            days_31_60_minor: 0,
            days_61_90_minor: 0,
            days_over_90_minor: 0,
        }];
        let ap = vec![ApAgingInput {
            currency: "USD".to_string(),
            current_minor: 100_000,
            days_1_30_minor: 0,
            days_31_60_minor: 0,
            days_61_90_minor: 0,
            over_90_minor: 0,
        }];
        let resp = compute_forecast(&ar, &ap, &[], &defaults(), vec![]);
        let f = &resp.forecasts[0];
        // inflow: 200_000 * 0.95 = 190_000
        // outflow: 100_000 * 1.0 = 100_000
        assert_eq!(f.total_net_minor, 190_000 - 100_000);
        assert_eq!(f.net_by_bucket.current_minor, 190_000 - 100_000);
    }

    #[test]
    fn multi_currency_separation() {
        let ar = vec![
            ArAgingInput {
                currency: "EUR".to_string(),
                current_minor: 50_000,
                ..ArAgingInput::default()
            },
            ArAgingInput {
                currency: "USD".to_string(),
                current_minor: 100_000,
                ..ArAgingInput::default()
            },
        ];
        let ap = vec![ApAgingInput {
            currency: "USD".to_string(),
            current_minor: 30_000,
            ..ApAgingInput::default()
        }];
        let resp = compute_forecast(&ar, &ap, &[], &defaults(), vec![]);
        assert_eq!(resp.forecasts.len(), 2);
        // Sorted by currency (BTreeMap)
        assert_eq!(resp.forecasts[0].currency, "EUR");
        assert_eq!(resp.forecasts[1].currency, "USD");
        // EUR: inflow only
        assert_eq!(resp.forecasts[0].inflows.current_minor, 47_500); // 50_000 * 0.95
        assert_eq!(resp.forecasts[0].outflows.total_minor, 0);
        // USD: inflow - outflow
        assert_eq!(resp.forecasts[1].inflows.current_minor, 95_000);
        assert_eq!(resp.forecasts[1].outflows.current_minor, 30_000);
    }

    #[test]
    fn scheduled_payments_reduce_net() {
        let ar = vec![ArAgingInput {
            currency: "USD".to_string(),
            current_minor: 100_000,
            ..ArAgingInput::default()
        }];
        let sched = vec![ScheduledPaymentInput {
            currency: "USD".to_string(),
            total_minor: 25_000,
        }];
        let resp = compute_forecast(&ar, &[], &sched, &defaults(), vec![]);
        let f = &resp.forecasts[0];
        // inflow: 95_000, scheduled outflow: 25_000
        assert_eq!(f.scheduled_outflows_minor, 25_000);
        assert_eq!(f.total_net_minor, 95_000 - 25_000);
    }

    #[test]
    fn all_aging_buckets_apply_decreasing_rates() {
        let ar = vec![ArAgingInput {
            currency: "USD".to_string(),
            current_minor: 100_000,
            days_1_30_minor: 100_000,
            days_31_60_minor: 100_000,
            days_61_90_minor: 100_000,
            days_over_90_minor: 100_000,
        }];
        let resp = compute_forecast(&ar, &[], &[], &defaults(), vec![]);
        let f = &resp.forecasts[0];
        // Each bucket should decrease: 95k > 85k > 70k > 50k > 25k
        assert_eq!(f.inflows.current_minor, 95_000);
        assert_eq!(f.inflows.days_1_30_minor, 85_000);
        assert_eq!(f.inflows.days_31_60_minor, 70_000);
        assert_eq!(f.inflows.days_61_90_minor, 50_000);
        assert_eq!(f.inflows.over_90_minor, 25_000);
        assert_eq!(f.inflows.total_minor, 325_000);
    }

    #[test]
    fn assumptions_included_in_response() {
        let resp = compute_forecast(&[], &[], &[], &defaults(), vec!["ar_aging".into()]);
        assert_eq!(resp.data_sources, vec!["ar_aging"]);
        assert_eq!(resp.assumptions.ar_current_rate, 0.95);
    }

    #[test]
    fn apply_rate_rounds_correctly() {
        // 333 * 0.95 = 316.35 → 316
        assert_eq!(apply_rate(333, 0.95), 316);
        // 1 * 0.5 = 0.5 → 1 (rounds up)
        assert_eq!(apply_rate(1, 0.5), 1);
        // 0 * anything = 0
        assert_eq!(apply_rate(0, 0.95), 0);
    }
}
