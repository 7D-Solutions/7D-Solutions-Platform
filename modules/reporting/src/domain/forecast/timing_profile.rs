//! Payment timing profile computation from `rpt_payment_history`.
//!
//! For each (tenant_id, customer_id, currency) with >= 3 history records,
//! builds an empirical CDF from sorted days_to_pay values.
//! Falls back to tenant-wide aggregate when a customer has < 3 records.

use sqlx::PgPool;
use std::collections::HashMap;

use super::types::PaymentProfile;

/// Minimum records needed to use per-customer profile.
const MIN_CUSTOMER_RECORDS: usize = 3;

/// Compute a payment profile for a specific customer+currency.
/// Returns `None` if fewer than `MIN_CUSTOMER_RECORDS` exist and no fallback is requested.
pub async fn compute_customer_profile(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: &str,
    currency: &str,
) -> Result<Option<PaymentProfile>, anyhow::Error> {
    let rows: Vec<(i32,)> = sqlx::query_as(
        r#"
        SELECT days_to_pay
        FROM rpt_payment_history
        WHERE tenant_id = $1 AND customer_id = $2 AND currency = $3
        ORDER BY days_to_pay ASC
        "#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(currency)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("customer profile query: {}", e))?;

    if rows.len() < MIN_CUSTOMER_RECORDS {
        return Ok(None);
    }

    let obs: Vec<i32> = rows.into_iter().map(|(d,)| d).collect();
    Ok(Some(build_profile(obs)))
}

/// Compute the tenant-wide fallback profile for a currency.
pub async fn compute_tenant_fallback(
    pool: &PgPool,
    tenant_id: &str,
    currency: &str,
) -> Result<Option<PaymentProfile>, anyhow::Error> {
    let rows: Vec<(i32,)> = sqlx::query_as(
        r#"
        SELECT days_to_pay
        FROM rpt_payment_history
        WHERE tenant_id = $1 AND currency = $2
        ORDER BY days_to_pay ASC
        "#,
    )
    .bind(tenant_id)
    .bind(currency)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("tenant fallback query: {}", e))?;

    if rows.is_empty() {
        return Ok(None);
    }

    let obs: Vec<i32> = rows.into_iter().map(|(d,)| d).collect();
    Ok(Some(build_profile(obs)))
}

/// Resolve profile: per-customer if >= 3 records, else tenant fallback.
pub async fn resolve_profile(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: &str,
    currency: &str,
) -> Result<Option<PaymentProfile>, anyhow::Error> {
    if let Some(p) = compute_customer_profile(pool, tenant_id, customer_id, currency).await? {
        return Ok(Some(p));
    }
    compute_tenant_fallback(pool, tenant_id, currency).await
}

/// Batch-load all profiles for open invoices in a tenant.
/// Returns map from (customer_id, currency) → PaymentProfile.
pub async fn load_profiles_for_tenant<'a>(
    pool: &PgPool,
    tenant_id: &str,
    pairs: &[(&'a str, &'a str)],
) -> Result<HashMap<(&'a str, &'a str), PaymentProfile>, anyhow::Error> {
    let mut result = HashMap::new();
    // Deduplicate (customer_id, currency) pairs
    let mut seen = std::collections::HashSet::new();
    for &(cid, cur) in pairs {
        if !seen.insert((cid, cur)) {
            continue;
        }
        if let Some(profile) = resolve_profile(pool, tenant_id, cid, cur).await? {
            result.insert((cid, cur), profile);
        }
    }
    Ok(result)
}

/// Build a PaymentProfile from sorted observations.
fn build_profile(obs: Vec<i32>) -> PaymentProfile {
    let p25 = percentile(&obs, 0.25);
    let p50 = percentile(&obs, 0.50);
    let p75 = percentile(&obs, 0.75);
    PaymentProfile {
        observations: obs,
        p25,
        p50,
        p75,
    }
}

/// Linear interpolation percentile (matches PERCENTILE_CONT behavior).
fn percentile(sorted: &[i32], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0] as f64;
    }
    let n = sorted.len() as f64;
    let idx = p * (n - 1.0);
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let frac = idx - lo as f64;
    sorted[lo] as f64 + frac * (sorted[hi] as f64 - sorted[lo] as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_single() {
        assert!((percentile(&[10], 0.5) - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_percentile_even() {
        // [10, 20, 30, 40] → p50 = 25.0 (linear interp)
        let v = vec![10, 20, 30, 40];
        let p50 = percentile(&v, 0.50);
        assert!((p50 - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_percentile_p25() {
        let v = vec![10, 20, 30, 40];
        let p25 = percentile(&v, 0.25);
        assert!((p25 - 17.5).abs() < 0.001);
    }

    #[test]
    fn test_percentile_p75() {
        let v = vec![10, 20, 30, 40];
        let p75 = percentile(&v, 0.75);
        assert!((p75 - 32.5).abs() < 0.001);
    }
}
