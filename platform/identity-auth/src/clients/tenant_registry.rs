//! HTTP client for the tenant-registry entitlements and status endpoints.
//!
//! Exposes:
//!   `get_concurrent_user_limit(tenant_id)` — concurrent_user_limit from:
//!     GET {base_url}/api/tenants/{tenant_id}/entitlements
//!   `get_tenant_status(tenant_id)` — lifecycle status from:
//!     GET {base_url}/api/tenants/{tenant_id}/status
//!
//! Fail-closed policy (login is denied when data cannot be determined):
//!   1. Cache hit (within TTL): return cached value immediately.
//!   2. Cache miss / expired: fetch from tenant-registry.
//!      - Fetch OK → update cache, return fresh value.
//!      - Fetch fail + stale cache within grace period → use stale value (outage tolerance).
//!      - Fetch fail + no usable cache → deny (return error).
//!
//! Tenant status policy:
//!   - trial / active → allow login and refresh
//!   - past_due → deny NEW logins (grace: allow refresh for 7 days after first past_due)
//!   - suspended / deleted → deny login and refresh
//!   - Registry unavailable + no cached status → deny (fail-closed)

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use platform_client_tenant_registry::TenantsClient;
use platform_sdk::PlatformClient;
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

// ---------------------------------------------------------------------------
// Tenant status gating
// ---------------------------------------------------------------------------

/// Result of a tenant lifecycle gate check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantGate {
    /// Allow: tenant is trial or active.
    Allow,
    /// Deny new login only; allow refresh (past_due within 7-day grace).
    DenyNewLogin { status: String },
    /// Deny both login and refresh.
    Deny { status: String },
}

/// Status fetch failed and no usable cached value exists.
/// Callers must deny (fail-closed).
#[derive(Debug)]
pub struct StatusUnavailable;

impl std::fmt::Display for StatusUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tenant-registry unavailable and no cached status")
    }
}

impl std::fmt::Display for EntitlementUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tenant-registry unavailable and no cached entitlement")
    }
}

// ---------------------------------------------------------------------------
// Cache entries
// ---------------------------------------------------------------------------

struct CachedEntry {
    limit: i64,
    cached_at: Instant,
}

struct CachedStatus {
    status: String,
    cached_at: Instant,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TenantRegistryClient {
    inner: Arc<TenantsClient>,
    /// Per-tenant entitlement TTL cache.  Arc because Clone on DashMap would deep-copy.
    cache: Arc<DashMap<Uuid, CachedEntry>>,
    /// Per-tenant status TTL cache.
    status_cache: Arc<DashMap<Uuid, CachedStatus>>,
    ttl: Duration,
    /// Grace period beyond TTL during which stale values are still usable.
    grace: Duration,
}

impl TenantRegistryClient {
    pub fn new(base_url: String, ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(TenantsClient::new(platform_sdk::PlatformClient::new(
                base_url,
            ))),
            cache: Arc::new(DashMap::new()),
            status_cache: Arc::new(DashMap::new()),
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
        let cached = self.cache.get(&tenant_id).map(|e| (e.limit, e.cached_at));

        if let Some((limit, cached_at)) = cached {
            if cached_at.elapsed() < self.ttl {
                metrics.auth_entitlement_cache_hit_total.inc();
                tracing::debug!(tenant_id = %tenant_id, limit, "entitlement.cache_hit");
                return Ok(limit);
            }
        }

        // --- 2. Fetch via typed client ---
        let svc_claims = PlatformClient::service_claims(tenant_id);
        match self.inner.get_entitlements(&svc_claims, tenant_id).await {
            Ok(row) => {
                let limit = row.concurrent_user_limit as i64;
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
                self.handle_fetch_failure(tenant_id, metrics, e.to_string())
            }
        }
    }

    /// Return the tenant lifecycle gate decision for login/refresh.
    ///
    /// Uses the same TTL + grace-period caching strategy as entitlements.
    /// Fail-closed: if status cannot be determined, returns `Err(StatusUnavailable)`.
    ///
    /// Gate policy:
    ///   - "trial" | "active"    → `Allow`
    ///   - "past_due"            → `DenyNewLogin` (refresh allowed during grace)
    ///   - "suspended" | "deleted" | anything else → `Deny`
    pub async fn get_tenant_gate(
        &self,
        tenant_id: Uuid,
        metrics: &Metrics,
    ) -> Result<TenantGate, StatusUnavailable> {
        // --- 1. Cache look-up ---
        let cached = self
            .status_cache
            .get(&tenant_id)
            .map(|e| (e.status.clone(), e.cached_at));

        if let Some((status, cached_at)) = cached {
            if cached_at.elapsed() < self.ttl {
                metrics.auth_tenant_status_cache_hit_total.inc();
                tracing::debug!(tenant_id = %tenant_id, status = %status, "tenant_status.cache_hit");
                return Ok(gate_from_status(&status));
            }
        }

        // --- 2. Fetch via typed client ---
        let svc_claims = PlatformClient::service_claims(tenant_id);
        match self.inner.get_tenant_status(&svc_claims, tenant_id).await {
            Ok(row) => {
                let status = row.status;
                self.status_cache.insert(
                    tenant_id,
                    CachedStatus {
                        status: status.clone(),
                        cached_at: Instant::now(),
                    },
                );
                metrics
                    .auth_tenant_status_fetch_total
                    .with_label_values(&["ok"])
                    .inc();
                tracing::info!(tenant_id = %tenant_id, status = %status, "tenant_status.fetch_ok");
                Ok(gate_from_status(&status))
            }
            Err(e) => {
                metrics
                    .auth_tenant_status_fetch_total
                    .with_label_values(&["fail"])
                    .inc();
                self.handle_status_fetch_failure(tenant_id, metrics, e.to_string())
            }
        }
    }

    /// Called when a live status fetch has failed.  Falls back to stale cache within grace.
    fn handle_status_fetch_failure(
        &self,
        tenant_id: Uuid,
        metrics: &Metrics,
        reason: String,
    ) -> Result<TenantGate, StatusUnavailable> {
        if let Some(entry) = self.status_cache.get(&tenant_id) {
            let elapsed = entry.cached_at.elapsed();
            if elapsed < self.ttl + self.grace {
                let status = entry.status.clone();
                tracing::warn!(
                    tenant_id = %tenant_id,
                    stale_age_secs = elapsed.as_secs(),
                    status = %status,
                    reason = %reason,
                    "tenant_status.using_stale_cache"
                );
                return Ok(gate_from_status(&status));
            }
        }

        metrics
            .auth_tenant_status_denied_total
            .with_label_values(&["unavailable"])
            .inc();
        tracing::warn!(
            tenant_id = %tenant_id,
            reason = %reason,
            "tenant_status.denied_no_usable_cache"
        );
        Err(StatusUnavailable)
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
// Gate decision helper
// ---------------------------------------------------------------------------

/// Map a lifecycle status string to a gate decision.
///
/// Policy:
///   trial | active    → Allow
///   past_due          → DenyNewLogin (refresh tolerated during billing grace)
///   suspended | deleted | provisioning | pending | failed → Deny
pub fn gate_from_status(status: &str) -> TenantGate {
    match status {
        "trial" | "active" => TenantGate::Allow,
        "past_due" => TenantGate::DenyNewLogin {
            status: status.to_string(),
        },
        other => TenantGate::Deny {
            status: other.to_string(),
        },
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

        let result = client.get_concurrent_user_limit(tenant_id, &metrics).await;
        assert_eq!(
            result.unwrap(),
            7,
            "should return cached limit without HTTP"
        );
    }

    #[tokio::test]
    async fn expired_cache_fail_closed_when_registry_down() {
        // Seed an entry older than TTL + grace → must fail closed.
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        let over_grace = Duration::from_secs(60 + GRACE_SECS + 1);
        seed_cache(&client, tenant_id, 5, over_grace);

        let result = client.get_concurrent_user_limit(tenant_id, &metrics).await;
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

        let result = client.get_concurrent_user_limit(tenant_id, &metrics).await;
        assert!(
            result.is_err(),
            "no cache + unreachable registry must be denied"
        );
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

        let result = client.get_concurrent_user_limit(tenant_id, &metrics).await;
        assert_eq!(
            result.unwrap(),
            3,
            "stale cache within grace must be used when registry is down"
        );
    }

    // -----------------------------------------------------------------------
    // Gate tests (no HTTP — test gate_from_status policy mapping)
    // -----------------------------------------------------------------------

    fn seed_status_cache(
        client: &TenantRegistryClient,
        tenant_id: Uuid,
        status: &str,
        age: Duration,
    ) {
        client.status_cache.insert(
            tenant_id,
            CachedStatus {
                status: status.to_string(),
                cached_at: Instant::now() - age,
            },
        );
    }

    #[test]
    fn gate_active_is_allow() {
        assert_eq!(gate_from_status("active"), TenantGate::Allow);
    }

    #[test]
    fn gate_trial_is_allow() {
        assert_eq!(gate_from_status("trial"), TenantGate::Allow);
    }

    #[test]
    fn gate_past_due_is_deny_new_login() {
        assert_eq!(
            gate_from_status("past_due"),
            TenantGate::DenyNewLogin {
                status: "past_due".to_string()
            }
        );
    }

    #[test]
    fn gate_suspended_is_deny() {
        assert_eq!(
            gate_from_status("suspended"),
            TenantGate::Deny {
                status: "suspended".to_string()
            }
        );
    }

    #[test]
    fn gate_deleted_is_deny() {
        assert_eq!(
            gate_from_status("deleted"),
            TenantGate::Deny {
                status: "deleted".to_string()
            }
        );
    }

    #[tokio::test]
    async fn status_cache_hit_within_ttl_returns_immediately() {
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        seed_status_cache(&client, tenant_id, "active", Duration::from_secs(0));

        let result = client.get_tenant_gate(tenant_id, &metrics).await;
        assert_eq!(
            result.unwrap(),
            TenantGate::Allow,
            "cache hit must return Allow"
        );
    }

    #[tokio::test]
    async fn status_no_cache_fail_closed_when_registry_down() {
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        let result = client.get_tenant_gate(tenant_id, &metrics).await;
        assert!(
            result.is_err(),
            "no cache + unreachable registry must fail closed"
        );
    }

    #[tokio::test]
    async fn status_stale_cache_within_grace_used_during_outage() {
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        // Past TTL (70s) but within grace (300s)
        seed_status_cache(&client, tenant_id, "active", Duration::from_secs(70));

        let result = client.get_tenant_gate(tenant_id, &metrics).await;
        assert_eq!(
            result.unwrap(),
            TenantGate::Allow,
            "stale cache within grace must be used"
        );
    }

    #[tokio::test]
    async fn status_expired_beyond_grace_fail_closed() {
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        let over_grace = Duration::from_secs(60 + GRACE_SECS + 1);
        seed_status_cache(&client, tenant_id, "active", over_grace);

        let result = client.get_tenant_gate(tenant_id, &metrics).await;
        assert!(
            result.is_err(),
            "cache beyond grace + unreachable registry must fail closed"
        );
    }

    #[tokio::test]
    async fn suspended_status_from_cache_returns_deny() {
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        seed_status_cache(&client, tenant_id, "suspended", Duration::from_secs(0));

        let result = client.get_tenant_gate(tenant_id, &metrics).await;
        assert!(
            matches!(result, Ok(TenantGate::Deny { .. })),
            "suspended tenant must be denied"
        );
    }

    #[tokio::test]
    async fn past_due_status_from_cache_returns_deny_new_login() {
        let client = make_client("http://127.0.0.1:19999");
        let metrics = Metrics::new();
        let tenant_id = Uuid::new_v4();

        seed_status_cache(&client, tenant_id, "past_due", Duration::from_secs(0));

        let result = client.get_tenant_gate(tenant_id, &metrics).await;
        assert!(
            matches!(result, Ok(TenantGate::DenyNewLogin { .. })),
            "past_due tenant must deny new logins only"
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
