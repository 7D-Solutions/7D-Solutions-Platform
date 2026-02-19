/// HTTP client for the tenant-registry entitlements endpoint.
///
/// Exposes `get_concurrent_user_limit(tenant_id)` which returns the
/// `concurrent_user_limit` from:
///   GET {base_url}/api/tenants/{tenant_id}/entitlements
///
/// Fail-closed policy (login is denied when limit cannot be determined):
///   1. Cache hit (within TTL): return cached value immediately.
///   2. Cache miss / expired: fetch from tenant-registry.
///      - Fetch OK → update cache, return fresh value.
///      - Fetch fail + stale cache within grace period → use stale value (outage tolerance).
///      - Fetch fail + no usable cache → deny (return error).

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::Deserialize;
use uuid::Uuid;

use crate::metrics::Metrics;

/// Stale cache tolerance when tenant-registry is down (5 min).
const GRACE_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Entitlement fetch failed and no usable cached value exists.
/// Callers must deny the login (fail-closed).
#[derive(Debug)]
pub struct EntitlementUnavailable;

impl std::fmt::Display for EntitlementUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tenant-registry unavailable and no cached entitlement")
    }
}

// ---------------------------------------------------------------------------
// Response shape (mirrors EntitlementRow in tenant-registry)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EntitlementResponse {
    concurrent_user_limit: i32,
}

// ---------------------------------------------------------------------------
// Cache entry
// ---------------------------------------------------------------------------

struct CachedEntry {
    limit: i64,
    cached_at: Instant,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TenantRegistryClient {
    http: reqwest::Client,
    base_url: String,
    /// Per-tenant TTL cache.  Arc because Clone on DashMap would deep-copy.
    cache: Arc<DashMap<Uuid, CachedEntry>>,
    ttl: Duration,
    /// Grace period beyond TTL during which stale values are still usable.
    grace: Duration,
}

impl TenantRegistryClient {
    pub fn new(base_url: String, ttl_secs: u64) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client for tenant-registry");

        Self {
            http,
            base_url,
            cache: Arc::new(DashMap::new()),
            ttl: Duration::from_secs(ttl_secs.max(1)),
            grace: Duration::from_secs(GRACE_SECS),
        }
    }

    /// Return `concurrent_user_limit` for the given tenant.
    ///
    /// Emits Prometheus metrics via `metrics`:
    ///   - `auth_entitlement_cache_hit_total`
    ///   - `auth_entitlement_fetch_total{result=ok|fail}`
    ///   - `auth_entitlement_denied_total{reason=no_cache}`
    pub async fn get_concurrent_user_limit(
        &self,
        tenant_id: Uuid,
        metrics: &Metrics,
    ) -> Result<i64, EntitlementUnavailable> {
        // --- 1. Cache look-up (drop guard before any await) ---
        let cached = self
            .cache
            .get(&tenant_id)
            .map(|e| (e.limit, e.cached_at));

        if let Some((limit, cached_at)) = cached {
            if cached_at.elapsed() < self.ttl {
                metrics.auth_entitlement_cache_hit_total.inc();
                tracing::debug!(tenant_id = %tenant_id, limit, "entitlement.cache_hit");
                return Ok(limit);
            }
        }

        // --- 2. Fetch from tenant-registry ---
        let url = format!(
            "{}/api/tenants/{}/entitlements",
            self.base_url, tenant_id
        );

        let fetch_result = self.http.get(&url).send().await;

        match fetch_result {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<EntitlementResponse>().await {
                    Ok(body) => {
                        let limit = body.concurrent_user_limit as i64;
                        self.cache.insert(
                            tenant_id,
                            CachedEntry {
                                limit,
                                cached_at: Instant::now(),
                            },
                        );
                        metrics
                            .auth_entitlement_fetch_total
                            .with_label_values(&["ok"])
                            .inc();
                        tracing::info!(tenant_id = %tenant_id, limit, "entitlement.fetch_ok");
                        Ok(limit)
                    }
                    Err(e) => {
                        metrics
                            .auth_entitlement_fetch_total
                            .with_label_values(&["fail"])
                            .inc();
                        self.handle_fetch_failure(
                            tenant_id,
                            metrics,
                            format!("json decode: {e}"),
                        )
                    }
                }
            }

            Ok(resp) => {
                let status = resp.status();
                metrics
                    .auth_entitlement_fetch_total
                    .with_label_values(&["fail"])
                    .inc();
                self.handle_fetch_failure(
                    tenant_id,
                    metrics,
                    format!("registry returned HTTP {status}"),
                )
            }

            Err(e) => {
                metrics
                    .auth_entitlement_fetch_total
                    .with_label_values(&["fail"])
                    .inc();
                self.handle_fetch_failure(tenant_id, metrics, format!("http error: {e}"))
            }
        }
    }

    /// Called when a live fetch has failed.
    ///
    /// Falls back to a stale cached value if within grace period,
    /// otherwise returns `UnavailableNoCachedValue` (fail-closed).
    fn handle_fetch_failure(
        &self,
        tenant_id: Uuid,
        metrics: &Metrics,
        reason: String,
    ) -> Result<i64, EntitlementUnavailable> {
        // Attempt stale cache within grace period
        if let Some(entry) = self.cache.get(&tenant_id) {
            let elapsed = entry.cached_at.elapsed();
            if elapsed < self.ttl + self.grace {
                let limit = entry.limit;
                tracing::warn!(
                    tenant_id = %tenant_id,
                    stale_age_secs = elapsed.as_secs(),
                    limit,
                    reason = %reason,
                    "entitlement.using_stale_cache"
                );
                return Ok(limit);
            }
        }

        // No usable cache — fail closed
        metrics
            .auth_entitlement_denied_total
            .with_label_values(&["no_cache"])
            .inc();
        tracing::warn!(
            tenant_id = %tenant_id,
            reason = %reason,
            "entitlement.denied_no_usable_cache"
        );
        Err(EntitlementUnavailable)
    }
}

// ---------------------------------------------------------------------------
// Tests (real HTTP behaviour — no mocks)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;

    fn make_client(base_url: &str) -> TenantRegistryClient {
        TenantRegistryClient::new(base_url.to_string(), 60)
    }

    // Pre-populate the cache to simulate a previous successful fetch.
    fn seed_cache(client: &TenantRegistryClient, tenant_id: Uuid, limit: i64, age: Duration) {
        client.cache.insert(
            tenant_id,
            CachedEntry {
                limit,
                cached_at: Instant::now() - age,
            },
        );
    }

    #[tokio::test]
    async fn cache_hit_within_ttl_returns_immediately() {
        // Point to an invalid URL — cache hit must not attempt HTTP.
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        // Seed a fresh cache entry
        seed_cache(&client, tenant_id, 7, Duration::from_secs(0));

        let result = client
            .get_concurrent_user_limit(tenant_id, &metrics)
            .await;
        assert_eq!(result.unwrap(), 7, "should return cached limit without HTTP");
    }

    #[tokio::test]
    async fn expired_cache_fail_closed_when_registry_down() {
        // Seed an entry older than TTL + grace → must fail closed.
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        let over_grace = Duration::from_secs(60 + GRACE_SECS + 1);
        seed_cache(&client, tenant_id, 5, over_grace);

        let result = client
            .get_concurrent_user_limit(tenant_id, &metrics)
            .await;
        assert!(
            result.is_err(),
            "should fail closed when stale entry exceeds grace period"
        );
    }

    #[tokio::test]
    async fn no_cache_fail_closed_when_registry_down() {
        // No cache at all — must fail closed immediately.
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        let result = client
            .get_concurrent_user_limit(tenant_id, &metrics)
            .await;
        assert!(result.is_err(), "no cache + unreachable registry must be denied");
    }

    #[tokio::test]
    async fn stale_cache_within_grace_used_during_outage() {
        // Seed an entry that is past TTL but within grace period.
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        // Expired TTL (70s) but within grace (300s)
        let age = Duration::from_secs(70);
        seed_cache(&client, tenant_id, 3, age);

        let result = client
            .get_concurrent_user_limit(tenant_id, &metrics)
            .await;
        assert_eq!(
            result.unwrap(),
            3,
            "stale cache within grace must be used when registry is down"
        );
    }

    /// Integration test: requires TENANT_REGISTRY_URL env var and a running
    /// control-plane with a tenant that has an entitlements row.
    ///
    /// Run with: TENANT_REGISTRY_URL=http://localhost:8092 \
    ///           TENANT_REGISTRY_TEST_TENANT_ID=<uuid> \
    ///           cargo test -p auth-rs entitlement_live_fetch -- --ignored
    #[tokio::test]
    #[ignore]
    async fn entitlement_live_fetch_from_registry() {
        let base_url = std::env::var("TENANT_REGISTRY_URL")
            .unwrap_or_else(|_| "http://localhost:8092".to_string());
        let tenant_id: Uuid = std::env::var("TENANT_REGISTRY_TEST_TENANT_ID")
            .expect("set TENANT_REGISTRY_TEST_TENANT_ID")
            .parse()
            .expect("valid UUID");

        let client = make_client(&base_url);
        let metrics = Metrics::new();

        let limit = client
            .get_concurrent_user_limit(tenant_id, &metrics)
            .await
            .expect("live fetch must succeed");
        assert!(limit > 0, "concurrent_user_limit must be positive");

        // Second call must be a cache hit
        let limit2 = client
            .get_concurrent_user_limit(tenant_id, &metrics)
            .await
            .expect("cache hit must succeed");
        assert_eq!(limit, limit2, "cache hit must return same limit");
    }
}
