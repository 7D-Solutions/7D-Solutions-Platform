//! Cash forecast computation — orchestrates profiles, probability, and open invoices.
//!
//! Groups results by currency. For each currency:
//!   - Fetches open invoices from rpt_open_invoices_cache
//!   - Resolves payment profiles (per-customer or tenant fallback)
//!   - Computes expected collection per horizon using conditional probability
//!   - Builds p25/p75 confidence scenarios
//!   - Identifies at-risk invoices (P(30) < 0.40)

use chrono::Utc;
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap};

use super::probability::compute_conditional_p;
use super::timing_profile::load_profiles_for_tenant;
use super::types::*;

/// Row from rpt_open_invoices_cache WHERE status='open'.
#[derive(Debug)]
struct OpenInvoice {
    invoice_id: String,
    customer_id: String,
    currency: String,
    amount_cents: i64,
    issued_at: chrono::DateTime<Utc>,
}

impl OpenInvoice {
    fn age_days(&self, now: chrono::DateTime<Utc>) -> i32 {
        (now.date_naive() - self.issued_at.date_naive())
            .num_days()
            .max(0) as i32
    }
}

/// Compute the full cash forecast for a tenant.
pub async fn compute_cash_forecast(
    pool: &PgPool,
    tenant_id: &str,
    horizons: &[u32],
) -> Result<CashForecastResponse, anyhow::Error> {
    let now = Utc::now();

    // 1. Load all open invoices
    let invoices = load_open_invoices(pool, tenant_id).await?;

    if invoices.is_empty() {
        return Ok(CashForecastResponse {
            as_of: now,
            results: vec![],
        });
    }

    // 2. Collect unique (customer_id, currency) pairs for profile resolution
    let pairs: Vec<(String, String)> = invoices
        .iter()
        .map(|inv| (inv.customer_id.clone(), inv.currency.clone()))
        .collect();

    let profiles = load_profiles_for_tenant(pool, tenant_id, &pairs).await?;

    // 3. Group invoices by currency
    let mut by_currency: BTreeMap<String, Vec<&OpenInvoice>> = BTreeMap::new();
    for inv in &invoices {
        by_currency
            .entry(inv.currency.clone())
            .or_default()
            .push(inv);
    }

    // 4. Compute forecast per currency
    let mut results = Vec::new();
    for (currency, currency_invoices) in &by_currency {
        let forecast = compute_currency_forecast(
            currency,
            currency_invoices,
            &profiles,
            horizons,
            now,
        );
        results.push(forecast);
    }

    Ok(CashForecastResponse {
        as_of: now,
        results,
    })
}

fn compute_currency_forecast(
    currency: &str,
    invoices: &[&OpenInvoice],
    profiles: &HashMap<(String, String), PaymentProfile>,
    horizons: &[u32],
    now: chrono::DateTime<Utc>,
) -> CurrencyForecast {
    let mut horizon_results = Vec::new();

    for &h in horizons {
        let mut expected_total: f64 = 0.0;
        let mut p25_total: i64 = 0;
        let mut p75_total: i64 = 0;

        for inv in invoices {
            let age = inv.age_days(now) as u32;
            let key = (inv.customer_id.clone(), inv.currency.clone());

            if let Some(profile) = profiles.get(&key) {
                // Expected: amount × conditional probability
                let p = compute_conditional_p(profile, age, h);
                expected_total += inv.amount_cents as f64 * p;

                // p25 scenario: invoice pays if (p25_days - age) <= horizon
                let remaining_p25 = (profile.p25 - age as f64).max(0.0);
                if remaining_p25 <= h as f64 {
                    p25_total += inv.amount_cents;
                }

                // p75 scenario: invoice pays if (p75_days - age) <= horizon
                let remaining_p75 = (profile.p75 - age as f64).max(0.0);
                if remaining_p75 <= h as f64 {
                    p75_total += inv.amount_cents;
                }
            }
            // No profile → invoice contributes 0 to all scenarios
        }

        horizon_results.push(ForecastHorizon {
            days: h,
            expected_cents: expected_total.round() as i64,
            p25_cents: p25_total,
            p75_cents: p75_total,
        });
    }

    // At-risk: P(30) < 0.40
    let mut at_risk = Vec::new();
    for inv in invoices {
        let age = inv.age_days(now) as u32;
        let key = (inv.customer_id.clone(), inv.currency.clone());

        let p30 = profiles
            .get(&key)
            .map(|profile| compute_conditional_p(profile, age, 30))
            .unwrap_or(0.0); // No profile → definitely at risk

        if p30 < 0.40 {
            at_risk.push(AtRiskItem {
                invoice_id: inv.invoice_id.clone(),
                customer_id: inv.customer_id.clone(),
                currency: inv.currency.clone(),
                amount_cents: inv.amount_cents,
                p30,
                age_days: inv.age_days(now),
            });
        }
    }
    at_risk.sort_by(|a, b| b.amount_cents.cmp(&a.amount_cents));

    CurrencyForecast {
        currency: currency.to_string(),
        horizons: horizon_results,
        at_risk,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serial_test::serial;
    use sqlx::PgPool;

    const TENANT: &str = "test-forecast-domain";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/reporting_test".into())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        sqlx::migrate!("./db/migrations").run(&pool).await.expect("migrate");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        for table in &["rpt_payment_history", "rpt_open_invoices_cache"] {
            sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
                .bind(TENANT)
                .execute(pool)
                .await
                .ok();
        }
    }

    /// Seed payment history: simulate an invoice that took `days` to pay.
    async fn seed_history(
        pool: &PgPool,
        customer_id: &str,
        invoice_id: &str,
        currency: &str,
        amount_cents: i64,
        days_to_pay: i32,
    ) {
        let issued = Utc::now() - Duration::days(90);
        let paid = issued + Duration::days(days_to_pay as i64);
        sqlx::query(
            r#"INSERT INTO rpt_payment_history
               (tenant_id, customer_id, invoice_id, currency, amount_cents,
                issued_at, paid_at, days_to_pay, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
               ON CONFLICT (tenant_id, invoice_id) DO NOTHING"#,
        )
        .bind(TENANT)
        .bind(customer_id)
        .bind(invoice_id)
        .bind(currency)
        .bind(amount_cents)
        .bind(issued)
        .bind(paid)
        .bind(days_to_pay)
        .execute(pool)
        .await
        .expect("seed history");
    }

    /// Seed an open invoice issued `age_days` ago.
    async fn seed_open_invoice(
        pool: &PgPool,
        invoice_id: &str,
        customer_id: &str,
        currency: &str,
        amount_cents: i64,
        age_days: i64,
    ) {
        let issued = Utc::now() - Duration::days(age_days);
        sqlx::query(
            r#"INSERT INTO rpt_open_invoices_cache
               (tenant_id, invoice_id, customer_id, currency, amount_cents,
                issued_at, status, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, 'open', NOW(), NOW())
               ON CONFLICT (tenant_id, invoice_id) DO NOTHING"#,
        )
        .bind(TENANT)
        .bind(invoice_id)
        .bind(customer_id)
        .bind(currency)
        .bind(amount_cents)
        .bind(issued)
        .execute(pool)
        .await
        .expect("seed open invoice");
    }

    #[tokio::test]
    #[serial]
    async fn test_empty_forecast() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let result = compute_cash_forecast(&pool, TENANT, &[7, 30]).await.unwrap();
        assert!(result.results.is_empty());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_customer_with_3_history_uses_per_customer_profile() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Customer cust-a: 3 invoices paid in 10, 20, 30 days
        seed_history(&pool, "cust-a", "hist-a1", "USD", 10000, 10).await;
        seed_history(&pool, "cust-a", "hist-a2", "USD", 10000, 20).await;
        seed_history(&pool, "cust-a", "hist-a3", "USD", 10000, 30).await;

        // Open invoice from cust-a, 5 days old, $500
        seed_open_invoice(&pool, "open-a1", "cust-a", "USD", 50000, 5).await;

        let result = compute_cash_forecast(&pool, TENANT, &[30]).await.unwrap();
        assert_eq!(result.results.len(), 1);
        let usd = &result.results[0];
        assert_eq!(usd.currency, "USD");

        // Conditional P(30 | age=5): F(35)/F(5) adjusted
        // Observations: [10,20,30]. F(5)=0/3=0, F(35)=3/3=1.0
        // P = (1.0 - 0.0) / (1 - 0.0) = 1.0
        // expected_cents = 50000 * 1.0 = 50000
        let h30 = &usd.horizons[0];
        assert_eq!(h30.days, 30);
        assert_eq!(h30.expected_cents, 50000);

        // Should NOT be at-risk since P(30) = 1.0
        assert!(usd.at_risk.is_empty());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_customer_with_fewer_than_3_uses_tenant_fallback() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Customer cust-b: only 1 history record (< 3 → falls back)
        seed_history(&pool, "cust-b", "hist-b1", "USD", 10000, 15).await;

        // Customer cust-fallback: 5 history records → tenant-wide pool
        seed_history(&pool, "cust-fallback", "hist-f1", "USD", 10000, 10).await;
        seed_history(&pool, "cust-fallback", "hist-f2", "USD", 10000, 20).await;
        seed_history(&pool, "cust-fallback", "hist-f3", "USD", 10000, 30).await;
        seed_history(&pool, "cust-fallback", "hist-f4", "USD", 10000, 40).await;
        seed_history(&pool, "cust-fallback", "hist-f5", "USD", 10000, 50).await;

        // Open invoice from cust-b (uses tenant fallback), age 0
        seed_open_invoice(&pool, "open-b1", "cust-b", "USD", 30000, 0).await;

        let result = compute_cash_forecast(&pool, TENANT, &[30]).await.unwrap();
        assert_eq!(result.results.len(), 1);
        let h30 = &result.results[0].horizons[0];

        // Tenant-wide profile: [10,15,20,30,40,50] (6 records, sorted).
        // F(0) = 0, F(30) = 4/6 = 0.667
        // P(30|age=0) = (0.667 - 0) / (1 - 0) = 0.667
        // expected = 30000 * 0.667 ≈ 20000
        assert!(h30.expected_cents > 0, "should use tenant fallback");
        assert!(h30.expected_cents < 30000, "should be partial, not full");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_conditional_probability_accounts_for_age() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Profile: pays in [10, 20, 30, 40, 50] days
        seed_history(&pool, "cust-c", "hist-c1", "USD", 10000, 10).await;
        seed_history(&pool, "cust-c", "hist-c2", "USD", 10000, 20).await;
        seed_history(&pool, "cust-c", "hist-c3", "USD", 10000, 30).await;
        seed_history(&pool, "cust-c", "hist-c4", "USD", 10000, 40).await;
        seed_history(&pool, "cust-c", "hist-c5", "USD", 10000, 50).await;

        // Fresh invoice (age 0) and aged invoice (age 30)
        seed_open_invoice(&pool, "open-c-fresh", "cust-c", "USD", 100000, 0).await;
        seed_open_invoice(&pool, "open-c-aged", "cust-c", "USD", 100000, 30).await;

        let result = compute_cash_forecast(&pool, TENANT, &[14]).await.unwrap();
        let h14 = &result.results[0].horizons[0];

        // Fresh (age=0, h=14): F(14)=1/5=0.2, F(0)=0 → P=0.2 → 20000
        // Aged (age=30, h=14): F(44)=4/5=0.8, F(30)=3/5=0.6 → P=(0.8-0.6)/(1-0.6)=0.5 → 50000
        // Total expected ≈ 70000
        assert!(
            h14.expected_cents > 60000 && h14.expected_cents < 80000,
            "expected ~70000, got {}",
            h14.expected_cents
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_at_risk_populated_for_low_p30() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Profile: all invoices pay in [5, 6, 7] days (very fast payer)
        seed_history(&pool, "cust-d", "hist-d1", "USD", 10000, 5).await;
        seed_history(&pool, "cust-d", "hist-d2", "USD", 10000, 6).await;
        seed_history(&pool, "cust-d", "hist-d3", "USD", 10000, 7).await;

        // An invoice 10 days old → all observations are <=10, F(10)=1.0
        // P(30|age=10) → F(10)=1.0 → returns 0.0 → definitely at risk
        seed_open_invoice(&pool, "open-d1", "cust-d", "USD", 80000, 10).await;

        let result = compute_cash_forecast(&pool, TENANT, &[30]).await.unwrap();
        let usd = &result.results[0];

        assert_eq!(usd.at_risk.len(), 1);
        assert_eq!(usd.at_risk[0].invoice_id, "open-d1");
        assert!((usd.at_risk[0].p30 - 0.0).abs() < 0.001);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_grouped() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // USD profile
        seed_history(&pool, "cust-e", "hist-e1", "USD", 10000, 10).await;
        seed_history(&pool, "cust-e", "hist-e2", "USD", 10000, 20).await;
        seed_history(&pool, "cust-e", "hist-e3", "USD", 10000, 30).await;

        // EUR profile
        seed_history(&pool, "cust-e", "hist-e4", "EUR", 10000, 15).await;
        seed_history(&pool, "cust-e", "hist-e5", "EUR", 10000, 25).await;
        seed_history(&pool, "cust-e", "hist-e6", "EUR", 10000, 35).await;

        seed_open_invoice(&pool, "open-e-usd", "cust-e", "USD", 50000, 0).await;
        seed_open_invoice(&pool, "open-e-eur", "cust-e", "EUR", 60000, 0).await;

        let result = compute_cash_forecast(&pool, TENANT, &[30]).await.unwrap();

        // Should have 2 currency groups
        assert_eq!(result.results.len(), 2);
        let currencies: Vec<&str> = result.results.iter().map(|r| r.currency.as_str()).collect();
        assert!(currencies.contains(&"USD"));
        assert!(currencies.contains(&"EUR"));

        cleanup(&pool).await;
    }
}

async fn load_open_invoices(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<OpenInvoice>, anyhow::Error> {
    let rows: Vec<(String, String, String, i64, chrono::DateTime<Utc>)> = sqlx::query_as(
        r#"
        SELECT invoice_id, customer_id, currency, amount_cents, issued_at
        FROM rpt_open_invoices_cache
        WHERE tenant_id = $1 AND status = 'open'
        ORDER BY amount_cents DESC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("load open invoices: {}", e))?;

    Ok(rows
        .into_iter()
        .map(
            |(invoice_id, customer_id, currency, amount_cents, issued_at)| OpenInvoice {
                invoice_id,
                customer_id,
                currency,
                amount_cents,
                issued_at,
            },
        )
        .collect())
}
