//! Rate limiting utilities
//!
//! Provides tenant-aware rate limiting with token bucket algorithm.
//! Supports different limits for normal reads and fallback paths.

use dashmap::DashMap;
use prometheus::{CounterVec, Opts, Registry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Errors that can occur during rate limiting
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("Rate limit exceeded for tenant {tenant_id}, path {path}: {requests}/{window:?}")]
    LimitExceeded {
        tenant_id: String,
        path: String,
        requests: u32,
        window: Duration,
    },

    #[error("Metrics error: {0}")]
    MetricsError(String),
}

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests allowed in the time window
    pub max_requests: u32,
    /// Time window for rate limiting
    pub window: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 100,
            window: Duration::from_secs(60),
        }
    }
}

impl RateLimitConfig {
    /// Create a new rate limit configuration
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
        }
    }

    /// Create a configuration for fallback paths (tighter limits)
    pub fn fallback() -> Self {
        Self {
            max_requests: 10, // 10x tighter than normal reads
            window: Duration::from_secs(60),
        }
    }

    /// Create a configuration for normal read paths
    pub fn normal_read() -> Self {
        Self {
            max_requests: 100,
            window: Duration::from_secs(60),
        }
    }
}

/// Token bucket state for a single rate limit key
#[derive(Debug, Clone)]
struct TokenBucket {
    /// Number of tokens currently available
    tokens: f64,
    /// Last time tokens were refilled
    last_refill: Instant,
    /// Maximum tokens (capacity)
    capacity: u32,
    /// Token refill rate per second
    refill_rate: f64,
}

impl TokenBucket {
    fn new(capacity: u32, window: Duration) -> Self {
        let refill_rate = capacity as f64 / window.as_secs_f64();
        Self {
            tokens: capacity as f64,
            last_refill: Instant::now(),
            capacity,
            refill_rate,
        }
    }

    /// Attempt to consume a token. Returns true if successful.
    fn consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.refill_rate;

        self.tokens = (self.tokens + new_tokens).min(self.capacity as f64);
        self.last_refill = now;
    }

    /// Get remaining tokens
    fn remaining(&self) -> u32 {
        self.tokens.floor() as u32
    }
}

/// Prometheus metrics for rate limiting
pub struct RateLimitMetrics {
    /// Counter for rate limit rejections
    rejections: CounterVec,
}

impl RateLimitMetrics {
    /// Create new rate limit metrics
    pub fn new(registry: &Registry) -> Result<Self, RateLimitError> {
        let rejections = CounterVec::new(
            Opts::new(
                "rate_limit_rejections_total",
                "Total number of requests rejected by rate limiting",
            ),
            &["tenant_id", "path", "limit_type"],
        )
        .map_err(|e| RateLimitError::MetricsError(e.to_string()))?;

        registry
            .register(Box::new(rejections.clone()))
            .map_err(|e| RateLimitError::MetricsError(e.to_string()))?;

        Ok(Self { rejections })
    }

    /// Record a rate limit rejection
    pub fn record_rejection(&self, tenant_id: &str, path: &str, limit_type: &str) {
        self.rejections
            .with_label_values(&[tenant_id, path, limit_type])
            .inc();
    }
}

/// Thread-safe rate limiter with tenant-aware quotas
pub struct RateLimiter {
    /// Per-key token buckets
    buckets: Arc<DashMap<String, TokenBucket>>,
    /// Configuration for normal read paths
    normal_config: RateLimitConfig,
    /// Configuration for fallback paths (tighter limits)
    fallback_config: RateLimitConfig,
    /// Prometheus metrics
    metrics: Option<Arc<RateLimitMetrics>>,
}

impl RateLimiter {
    /// Create a new rate limiter with default configs
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            normal_config: RateLimitConfig::normal_read(),
            fallback_config: RateLimitConfig::fallback(),
            metrics: None,
        }
    }

    /// Create a new rate limiter with custom configs
    pub fn with_configs(normal: RateLimitConfig, fallback: RateLimitConfig) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            normal_config: normal,
            fallback_config: fallback,
            metrics: None,
        }
    }

    /// Attach Prometheus metrics
    pub fn with_metrics(mut self, metrics: Arc<RateLimitMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Build a rate limit key from tenant ID and path
    fn build_key(tenant_id: &str, path: &str, is_fallback: bool) -> String {
        let suffix = if is_fallback { "fallback" } else { "normal" };
        let mut key = String::with_capacity(tenant_id.len() + 1 + path.len() + 1 + suffix.len());
        key.push_str(tenant_id);
        key.push(':');
        key.push_str(path);
        key.push(':');
        key.push_str(suffix);
        key
    }

    /// Check if a request is allowed (normal read path)
    pub fn check_limit(&self, tenant_id: &str, path: &str) -> Result<(), crate::SecurityError> {
        self.check_limit_internal(tenant_id, path, false)
            .map_err(|_| crate::SecurityError::RateLimitExceeded)
    }

    /// Check if a fallback request is allowed (tighter limits)
    pub fn check_fallback_limit(
        &self,
        tenant_id: &str,
        path: &str,
    ) -> Result<(), crate::SecurityError> {
        self.check_limit_internal(tenant_id, path, true)
            .map_err(|_| crate::SecurityError::RateLimitExceeded)
    }

    /// Internal rate limit check
    fn check_limit_internal(
        &self,
        tenant_id: &str,
        path: &str,
        is_fallback: bool,
    ) -> Result<(), RateLimitError> {
        let config = if is_fallback {
            &self.fallback_config
        } else {
            &self.normal_config
        };

        let key = Self::build_key(tenant_id, path, is_fallback);

        // Fast path: bucket already exists — no key ownership transfer needed
        let consumed = if let Some(mut bucket) = self.buckets.get_mut(&key) {
            bucket.consume()
        } else {
            // Slow path: first request for this key — create bucket (consumes key)
            self.buckets
                .entry(key)
                .or_insert_with(|| TokenBucket::new(config.max_requests, config.window))
                .consume()
        };

        if consumed {
            Ok(())
        } else {
            if let Some(metrics) = &self.metrics {
                let limit_type = if is_fallback { "fallback" } else { "normal" };
                metrics.record_rejection(tenant_id, path, limit_type);
            }

            Err(RateLimitError::LimitExceeded {
                tenant_id: tenant_id.to_string(),
                path: path.to_string(),
                requests: config.max_requests,
                window: config.window,
            })
        }
    }

    /// Get remaining quota for a tenant/path (normal read)
    pub fn remaining_quota(&self, tenant_id: &str, path: &str) -> u32 {
        let key = Self::build_key(tenant_id, path, false);
        self.buckets
            .get(&key)
            .map(|bucket| bucket.remaining())
            .unwrap_or(self.normal_config.max_requests)
    }

    /// Get remaining fallback quota for a tenant/path
    pub fn remaining_fallback_quota(&self, tenant_id: &str, path: &str) -> u32 {
        let key = Self::build_key(tenant_id, path, true);
        self.buckets
            .get(&key)
            .map(|bucket| bucket.remaining())
            .unwrap_or(self.fallback_config.max_requests)
    }

    /// Clear all rate limit state (useful for testing)
    pub fn reset(&self) {
        self.buckets.clear();
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

// ── TieredRateLimiter ─────────────────────────────────────────────────────────

/// Strategy for building the rate limit key in a tiered rate limiter.
///
/// Determines which request attributes contribute to the per-bucket identity.
/// Select a strategy per tier based on whether the tier serves authenticated
/// multi-tenant traffic, public IP-gated endpoints, or something in between.
///
/// # Choosing a strategy
///
/// | Strategy      | Key includes        | Isolates per        | Use for                           |
/// |---------------|---------------------|---------------------|-----------------------------------|
/// | `Composite`   | tenant + IP         | (tenant, IP) pair   | Authenticated multi-tenant APIs   |
/// | `TenantOnly`  | tenant ID           | tenant              | Per-tenant quotas, any IP          |
/// | `IpOnly`      | IP address          | IP                  | Public/unauthenticated endpoints  |
///
/// `Composite` is the default. It prevents one tenant's traffic from exhausting
/// another tenant's quota **and** isolates different IPs within the same tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RateLimitKeyStrategy {
    /// Rate limit by the combination of tenant ID and IP address.
    ///
    /// Each `(tenant_id, ip)` pair gets its own independent token bucket.
    /// This is the default for multi-tenant modules.
    #[default]
    Composite,
    /// Rate limit by IP address only.
    ///
    /// All tenants behind the same IP share a single bucket. Use for
    /// public-facing endpoints where tenant context is unavailable.
    IpOnly,
    /// Rate limit by tenant ID only.
    ///
    /// All client IPs for a given tenant share one bucket. Use when you
    /// want per-tenant quotas regardless of how many IPs the tenant uses.
    TenantOnly,
}

struct TierState {
    config: RateLimitConfig,
    buckets: Arc<DashMap<String, TokenBucket>>,
    strategy: RateLimitKeyStrategy,
}

/// Multi-tier rate limiter with per-tier token buckets.
///
/// Each tier is fully isolated — a slow "login" tier cannot block fast "api"
/// tier lookups because each tier owns its own `DashMap`.
///
/// The rate limit key is derived from the tier's [`RateLimitKeyStrategy`]:
/// - `Composite` (default): `tier_name:tenant_id:ip`
/// - `IpOnly`: `tier_name:ip`
/// - `TenantOnly`: `tier_name:tenant_id`
///
/// Route assignment uses longest-prefix matching. Paths not matching any
/// configured prefix fall through to the default `"api"` tier.
pub struct TieredRateLimiter {
    tiers: std::collections::HashMap<String, TierState>,
    /// `(path_prefix, tier_name)` sorted longest-first for greedy match.
    route_map: Vec<(String, String)>,
    default_tier_name: String,
}

impl TieredRateLimiter {
    /// Build a tiered rate limiter from named tier definitions.
    ///
    /// Each entry is `(tier_name, config, route_prefixes)`.
    ///
    /// All tiers use the [`RateLimitKeyStrategy::Composite`] strategy (keyed
    /// on both tenant ID and IP). Use [`with_strategies`](Self::with_strategies)
    /// to configure per-tier strategies explicitly.
    ///
    /// If no tier named `"api"` is provided, a default `"api"` tier using
    /// [`RateLimitConfig::normal_read`] is inserted automatically.
    pub fn new(tiers: Vec<(String, RateLimitConfig, Vec<String>)>) -> Self {
        Self::with_strategies(
            tiers
                .into_iter()
                .map(|(n, c, r)| (n, c, r, RateLimitKeyStrategy::Composite))
                .collect(),
        )
    }

    /// Build a tiered rate limiter with an explicit key strategy per tier.
    ///
    /// Each entry is `(tier_name, config, route_prefixes, key_strategy)`.
    ///
    /// If no tier named `"api"` is provided, a default `"api"` tier using
    /// [`RateLimitConfig::normal_read`] and [`RateLimitKeyStrategy::Composite`]
    /// is inserted automatically.
    pub fn with_strategies(
        tiers: Vec<(String, RateLimitConfig, Vec<String>, RateLimitKeyStrategy)>,
    ) -> Self {
        let mut tier_map: std::collections::HashMap<String, TierState> =
            std::collections::HashMap::new();
        let mut route_map: Vec<(String, String)> = Vec::new();
        let default_tier_name = "api".to_string();

        for (name, config, routes, strategy) in tiers {
            for route in &routes {
                route_map.push((route.clone(), name.clone()));
            }
            tier_map.insert(
                name,
                TierState {
                    config,
                    buckets: Arc::new(DashMap::new()),
                    strategy,
                },
            );
        }

        // Ensure a default "api" tier always exists.
        if !tier_map.contains_key(&default_tier_name) {
            tier_map.insert(
                default_tier_name.clone(),
                TierState {
                    config: RateLimitConfig::normal_read(),
                    buckets: Arc::new(DashMap::new()),
                    strategy: RateLimitKeyStrategy::Composite,
                },
            );
        }

        // Longest-first so more-specific prefixes win.
        route_map.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Self {
            tiers: tier_map,
            route_map,
            default_tier_name,
        }
    }

    fn resolve_tier(&self, path: &str) -> &str {
        for (prefix, tier_name) in &self.route_map {
            if path.starts_with(prefix.as_str()) {
                return tier_name.as_str();
            }
        }
        self.default_tier_name.as_str()
    }

    fn build_key(
        tier_name: &str,
        tenant_id: &str,
        ip: &str,
        strategy: RateLimitKeyStrategy,
    ) -> String {
        match strategy {
            RateLimitKeyStrategy::Composite => {
                let mut key = String::with_capacity(
                    tier_name.len() + 1 + tenant_id.len() + 1 + ip.len(),
                );
                key.push_str(tier_name);
                key.push(':');
                key.push_str(tenant_id);
                key.push(':');
                key.push_str(ip);
                key
            }
            RateLimitKeyStrategy::IpOnly => {
                let mut key = String::with_capacity(tier_name.len() + 1 + ip.len());
                key.push_str(tier_name);
                key.push(':');
                key.push_str(ip);
                key
            }
            RateLimitKeyStrategy::TenantOnly => {
                let mut key =
                    String::with_capacity(tier_name.len() + 1 + tenant_id.len());
                key.push_str(tier_name);
                key.push(':');
                key.push_str(tenant_id);
                key
            }
        }
    }

    /// Check whether a request is within its tier's rate limit.
    ///
    /// - `path` — request URI path (used for tier resolution)
    /// - `tenant_id` — tenant identifier (from JWT claims, or IP as fallback)
    /// - `ip` — client IP (from `X-Forwarded-For` or peer addr)
    pub fn check_limit(
        &self,
        path: &str,
        tenant_id: &str,
        ip: &str,
    ) -> Result<(), crate::SecurityError> {
        let tier_name = self.resolve_tier(path);

        let tier = match self.tiers.get(tier_name) {
            Some(t) => t,
            None => return Ok(()),
        };

        let key = Self::build_key(tier_name, tenant_id, ip, tier.strategy);

        let consumed = if let Some(mut bucket) = tier.buckets.get_mut(&key) {
            bucket.consume()
        } else {
            tier.buckets
                .entry(key)
                .or_insert_with(|| TokenBucket::new(tier.config.max_requests, tier.config.window))
                .consume()
        };

        if consumed {
            Ok(())
        } else {
            Err(crate::SecurityError::RateLimitExceeded)
        }
    }

    /// Get the remaining quota for a path/tenant/ip combination.
    pub fn remaining_quota(&self, path: &str, tenant_id: &str, ip: &str) -> u32 {
        let tier_name = self.resolve_tier(path);
        match self.tiers.get(tier_name) {
            Some(tier) => {
                let key = Self::build_key(tier_name, tenant_id, ip, tier.strategy);
                tier.buckets
                    .get(&key)
                    .map(|b| b.remaining())
                    .unwrap_or(tier.config.max_requests)
            }
            None => 0,
        }
    }

    /// Clear all rate limit state (useful for testing).
    pub fn reset(&self) {
        for tier in self.tiers.values() {
            tier.buckets.clear();
        }
    }
}

/// IP-based rate limiter for inbound webhook endpoints.
///
/// Webhook endpoints are public-facing (no auth), so rate limiting by tenant
/// is not applicable. Instead, we limit by source IP to prevent flooding while
/// still allowing normal provider retry cadences.
///
/// Default: 120 requests per minute per IP (generous for legit providers).
pub struct WebhookRateLimiter {
    buckets: Arc<DashMap<String, TokenBucket>>,
    config: RateLimitConfig,
}

impl WebhookRateLimiter {
    /// Create with default config (120 req/min per IP).
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            config: RateLimitConfig::new(120, Duration::from_secs(60)),
        }
    }

    /// Create with custom config.
    pub fn with_config(config: RateLimitConfig) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            config,
        }
    }

    /// Check whether the IP is within its rate limit.
    ///
    /// Returns `Ok(())` if allowed, `Err(SecurityError::RateLimitExceeded)` if not.
    pub fn check_webhook_limit(&self, ip: &str) -> Result<(), crate::SecurityError> {
        // Fast path: bucket already exists — no allocation needed
        let consumed = if let Some(mut bucket) = self.buckets.get_mut(ip) {
            bucket.consume()
        } else {
            // Slow path: first request from this IP — allocate key
            self.buckets
                .entry(ip.to_string())
                .or_insert_with(|| TokenBucket::new(self.config.max_requests, self.config.window))
                .consume()
        };

        if consumed {
            Ok(())
        } else {
            Err(crate::SecurityError::RateLimitExceeded)
        }
    }

    /// Clear all buckets (useful in tests).
    pub fn reset(&self) {
        self.buckets.clear();
    }
}

impl Default for WebhookRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ratelimit_allows_within_quota() {
        let limiter = RateLimiter::with_configs(
            RateLimitConfig::new(5, Duration::from_secs(60)),
            RateLimitConfig::new(2, Duration::from_secs(60)),
        );

        // Should allow first 5 requests
        for _ in 0..5 {
            assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
        }

        // 6th request should be rejected
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_err());
    }

    #[test]
    fn test_ratelimit_tenant_isolation() {
        let limiter = RateLimiter::with_configs(
            RateLimitConfig::new(3, Duration::from_secs(60)),
            RateLimitConfig::new(1, Duration::from_secs(60)),
        );

        // Tenant 1 exhausts quota
        for _ in 0..3 {
            assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
        }
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_err());

        // Tenant 2 should still have full quota
        assert!(limiter.check_limit("tenant2", "/api/invoices").is_ok());
        assert!(limiter.check_limit("tenant2", "/api/invoices").is_ok());
        assert!(limiter.check_limit("tenant2", "/api/invoices").is_ok());
    }

    #[test]
    fn test_fallback_has_tighter_limits() {
        let limiter = RateLimiter::with_configs(
            RateLimitConfig::new(100, Duration::from_secs(60)),
            RateLimitConfig::new(10, Duration::from_secs(60)),
        );

        // Normal reads have 100 quota
        assert_eq!(limiter.remaining_quota("tenant1", "/api/invoices"), 100);

        // Fallback reads have only 10 quota
        assert_eq!(
            limiter.remaining_fallback_quota("tenant1", "/api/invoices"),
            10
        );
    }

    #[test]
    fn test_remaining_quota() {
        let limiter = RateLimiter::with_configs(
            RateLimitConfig::new(10, Duration::from_secs(60)),
            RateLimitConfig::new(5, Duration::from_secs(60)),
        );

        assert_eq!(limiter.remaining_quota("tenant1", "/api/invoices"), 10);

        // Consume 3 tokens
        for _ in 0..3 {
            assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
        }

        assert_eq!(limiter.remaining_quota("tenant1", "/api/invoices"), 7);
    }

    #[tokio::test]
    async fn test_token_refill_over_time() {
        let limiter = RateLimiter::with_configs(
            RateLimitConfig::new(2, Duration::from_millis(200)), // 2 tokens per 200ms = 10/sec
            RateLimitConfig::new(1, Duration::from_millis(200)),
        );

        // Exhaust quota
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_err());

        // Wait for refill
        tokio::time::sleep(Duration::from_millis(250)).await;

        // Should have refilled ~2.5 tokens, so at least 2 requests should work
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
        assert!(limiter.check_limit("tenant1", "/api/invoices").is_ok());
    }

    #[test]
    fn test_webhook_ratelimiter_allows_within_quota() {
        let limiter =
            WebhookRateLimiter::with_config(RateLimitConfig::new(3, Duration::from_secs(60)));
        assert!(limiter.check_webhook_limit("1.2.3.4").is_ok());
        assert!(limiter.check_webhook_limit("1.2.3.4").is_ok());
        assert!(limiter.check_webhook_limit("1.2.3.4").is_ok());
        assert!(limiter.check_webhook_limit("1.2.3.4").is_err());
    }

    #[test]
    fn test_webhook_ratelimiter_ip_isolation() {
        let limiter =
            WebhookRateLimiter::with_config(RateLimitConfig::new(2, Duration::from_secs(60)));
        // Exhaust IP 1
        assert!(limiter.check_webhook_limit("10.0.0.1").is_ok());
        assert!(limiter.check_webhook_limit("10.0.0.1").is_ok());
        assert!(limiter.check_webhook_limit("10.0.0.1").is_err());
        // IP 2 still has full quota
        assert!(limiter.check_webhook_limit("10.0.0.2").is_ok());
        assert!(limiter.check_webhook_limit("10.0.0.2").is_ok());
    }

    // ── TieredRateLimiter tests ────────────────────────────────────────────────

    fn build_tiered() -> TieredRateLimiter {
        TieredRateLimiter::new(vec![
            (
                "login".to_string(),
                RateLimitConfig::new(3, Duration::from_secs(60)),
                vec!["/api/auth/".to_string(), "/api/login".to_string()],
            ),
            (
                "api".to_string(),
                RateLimitConfig::new(10, Duration::from_secs(60)),
                vec!["/api/".to_string()],
            ),
        ])
    }

    #[test]
    fn test_tiered_different_tiers_enforce_different_limits() {
        let limiter = build_tiered();

        // "login" tier allows 3 requests, then blocks
        for _ in 0..3 {
            assert!(
                limiter.check_limit("/api/auth/token", "t1", "1.2.3.4").is_ok(),
                "login tier should allow request within quota"
            );
        }
        assert!(
            limiter
                .check_limit("/api/auth/token", "t1", "1.2.3.4")
                .is_err(),
            "login tier should block after quota exhausted"
        );

        // "api" tier is independent — same tenant, same IP still has 10 quota
        for _ in 0..10 {
            assert!(
                limiter.check_limit("/api/invoices", "t1", "1.2.3.4").is_ok(),
                "api tier should allow request within its own quota"
            );
        }
        assert!(
            limiter
                .check_limit("/api/invoices", "t1", "1.2.3.4")
                .is_err(),
            "api tier should block after its own quota exhausted"
        );
    }

    #[test]
    fn test_tiered_isolation_by_tenant_id() {
        let limiter = build_tiered();

        // Exhaust tenant1's login quota
        for _ in 0..3 {
            assert!(limiter
                .check_limit("/api/auth/token", "tenant1", "1.2.3.4")
                .is_ok());
        }
        assert!(limiter
            .check_limit("/api/auth/token", "tenant1", "1.2.3.4")
            .is_err());

        // tenant2 with same IP is unaffected
        assert!(
            limiter
                .check_limit("/api/auth/token", "tenant2", "1.2.3.4")
                .is_ok(),
            "different tenant should have independent quota"
        );
    }

    #[test]
    fn test_tiered_isolation_by_ip() {
        let limiter = build_tiered();

        // Exhaust 1.2.3.4's login quota
        for _ in 0..3 {
            assert!(limiter
                .check_limit("/api/auth/token", "t1", "1.2.3.4")
                .is_ok());
        }
        assert!(limiter
            .check_limit("/api/auth/token", "t1", "1.2.3.4")
            .is_err());

        // Same tenant, different IP — independent bucket
        assert!(
            limiter
                .check_limit("/api/auth/token", "t1", "9.9.9.9")
                .is_ok(),
            "different IP should have independent quota"
        );
    }

    #[test]
    fn test_tiered_default_api_tier_for_unmatched_paths() {
        let limiter = build_tiered();

        // /v1/orders does not match any explicit prefix → falls to "api" tier (10 limit)
        for _ in 0..10 {
            assert!(
                limiter.check_limit("/v1/orders", "t1", "1.2.3.4").is_ok(),
                "unmatched path should use default api tier"
            );
        }
        assert!(limiter
            .check_limit("/v1/orders", "t1", "1.2.3.4")
            .is_err());
    }

    #[test]
    fn test_tiered_longest_prefix_wins() {
        // /api/auth/ (3 limit) should win over /api/ (10 limit) for /api/auth/token
        let limiter = build_tiered();
        for _ in 0..3 {
            assert!(limiter
                .check_limit("/api/auth/token", "t1", "1.2.3.4")
                .is_ok());
        }
        // Should be blocked by "login" tier (limit=3), not "api" (limit=10)
        assert!(limiter
            .check_limit("/api/auth/token", "t1", "1.2.3.4")
            .is_err());
    }

    #[test]
    fn test_tiered_auto_default_tier_inserted_when_missing() {
        // Tier config with no "api" tier — should still work via auto-inserted default
        let limiter = TieredRateLimiter::new(vec![(
            "login".to_string(),
            RateLimitConfig::new(2, Duration::from_secs(60)),
            vec!["/api/auth/".to_string()],
        )]);

        // Unmatched paths go to auto "api" tier with normal_read defaults (100)
        assert!(limiter
            .check_limit("/api/invoices", "t1", "1.2.3.4")
            .is_ok());
    }

    #[test]
    fn test_tiered_remaining_quota() {
        let limiter = build_tiered();

        assert_eq!(
            limiter.remaining_quota("/api/auth/token", "t1", "1.2.3.4"),
            3,
            "fresh login tier should report full quota"
        );

        limiter
            .check_limit("/api/auth/token", "t1", "1.2.3.4")
            .expect("test assertion");
        assert_eq!(
            limiter.remaining_quota("/api/auth/token", "t1", "1.2.3.4"),
            2,
            "remaining quota should decrease after one request"
        );
    }

    #[test]
    fn test_tiered_reset_clears_all_tiers() {
        let limiter = build_tiered();

        // Exhaust both tiers
        for _ in 0..3 {
            let _ = limiter.check_limit("/api/auth/token", "t1", "1.2.3.4");
        }
        for _ in 0..10 {
            let _ = limiter.check_limit("/api/invoices", "t1", "1.2.3.4");
        }

        limiter.reset();

        assert!(limiter
            .check_limit("/api/auth/token", "t1", "1.2.3.4")
            .is_ok());
        assert!(limiter
            .check_limit("/api/invoices", "t1", "1.2.3.4")
            .is_ok());
    }
}
