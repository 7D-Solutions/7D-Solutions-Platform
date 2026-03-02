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
        format!(
            "{}:{}:{}",
            tenant_id,
            path,
            if is_fallback { "fallback" } else { "normal" }
        )
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
        let key = Self::build_key(tenant_id, path, is_fallback);
        let config = if is_fallback {
            &self.fallback_config
        } else {
            &self.normal_config
        };

        // Get or create token bucket for this key
        let mut bucket_ref = self
            .buckets
            .entry(key.clone())
            .or_insert_with(|| TokenBucket::new(config.max_requests, config.window));

        // Try to consume a token
        if bucket_ref.consume() {
            Ok(())
        } else {
            // Record rejection metric
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
        let mut bucket = self
            .buckets
            .entry(ip.to_string())
            .or_insert_with(|| TokenBucket::new(self.config.max_requests, self.config.window));

        if bucket.consume() {
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
            limiter.check_limit("tenant1", "/api/invoices").unwrap();
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
}
