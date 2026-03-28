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
///
/// Loads all payment history for the tenant in a single query, then builds
/// per-customer profiles in-memory. Falls back to tenant-wide aggregate
/// for customers with fewer than MIN_CUSTOMER_RECORDS.
pub async fn load_profiles_for_tenant<'a>(
    pool: &PgPool,
    tenant_id: &str,
    pairs: &[(&'a str, &'a str)],
) -> Result<HashMap<(&'a str, &'a str), PaymentProfile>, anyhow::Error> {
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }

    // Single query: load all payment history for this tenant, sorted for profile building
    let all_rows: Vec<(String, String, i32)> = sqlx::query_as(
        r#"
        SELECT customer_id, currency, days_to_pay
        FROM rpt_payment_history
        WHERE tenant_id = $1
        ORDER BY customer_id, currency, days_to_pay ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("batch profile query: {}", e))?;

    // Collect tenant-wide observations per currency for fallback
    let mut tenant_by_currency: HashMap<&str, Vec<i32>> = HashMap::new();

    // We need owned strings from rows but borrow them for the duration of this function.
    // Store rows so we can reference their strings.
    for row in &all_rows {
        tenant_by_currency
            .entry(row.1.as_str())
            .or_default()
            .push(row.2);
    }

    // Build per-customer lookup — we need to match against the borrowed pair keys
    let mut customer_obs: HashMap<(String, String), Vec<i32>> = HashMap::new();
    for row in &all_rows {
        customer_obs
            .entry((row.0.clone(), row.1.clone()))
            .or_default()
            .push(row.2);
    }

    // Deduplicate requested pairs
    let mut seen = std::collections::HashSet::new();
    let mut result = HashMap::new();

    for &(cid, cur) in pairs {
        if !seen.insert((cid, cur)) {
            continue;
        }

        // Try per-customer profile (>= 3 records)
        let key = (cid.to_string(), cur.to_string());
        if let Some(obs) = customer_obs.get(&key) {
            if obs.len() >= MIN_CUSTOMER_RECORDS {
                result.insert((cid, cur), build_profile(obs.clone()));
                continue;
            }
        }

        // Fallback to tenant-wide profile for this currency
        if let Some(obs) = tenant_by_currency.get(cur) {
            if !obs.is_empty() {
                result.insert((cid, cur), build_profile(obs.clone()));
            }
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
