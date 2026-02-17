//! E2E test for rate limiting and overload protection
//!
//! This test verifies that:
//! 1. Tenant-aware rate limits exist and engage properly
//! 2. Fallback paths have tighter limits than normal reads
//! 3. System remains healthy under rate limiting
//! 4. One tenant's overload doesn't affect other tenants (isolation)

use security::ratelimit::{RateLimiter, RateLimitConfig};
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_rate_limit_tenant_isolation() {
    // Create rate limiter with low quotas for testing
    let limiter = RateLimiter::with_configs(
        RateLimitConfig::new(5, Duration::from_secs(60)),  // 5 normal requests/min
        RateLimitConfig::new(2, Duration::from_secs(60)),  // 2 fallback requests/min
    );

    // Tenant 1 exhausts their quota
    for i in 0..5 {
        let result = limiter.check_limit("tenant-1", "/api/invoices");
        assert!(result.is_ok(), "Tenant 1 request {} should succeed", i + 1);
    }

    // Tenant 1's 6th request should be rate limited
    let result = limiter.check_limit("tenant-1", "/api/invoices");
    assert!(result.is_err(), "Tenant 1 should be rate limited after 5 requests");

    // Tenant 2 should still have full quota (isolation)
    for i in 0..5 {
        let result = limiter.check_limit("tenant-2", "/api/invoices");
        assert!(
            result.is_ok(),
            "Tenant 2 request {} should succeed (tenant isolation)",
            i + 1
        );
    }

    println!("✓ Tenant isolation verified: tenant-1 rate limited, tenant-2 unaffected");
}

#[tokio::test]
async fn test_fallback_has_tighter_limits() {
    // Create rate limiter
    let limiter = RateLimiter::with_configs(
        RateLimitConfig::new(100, Duration::from_secs(60)),  // 100 normal requests/min
        RateLimitConfig::new(10, Duration::from_secs(60)),   // 10 fallback requests/min (10x tighter)
    );

    // Check initial quotas
    let normal_quota = limiter.remaining_quota("tenant-1", "/api/invoices");
    let fallback_quota = limiter.remaining_fallback_quota("tenant-1", "/api/invoices");

    assert_eq!(normal_quota, 100, "Normal quota should be 100");
    assert_eq!(fallback_quota, 10, "Fallback quota should be 10 (10x tighter)");

    // Exhaust normal quota (would take 100 requests)
    // Instead, just exhaust fallback quota (10 requests)
    for i in 0..10 {
        let result = limiter.check_fallback_limit("tenant-1", "/api/invoices");
        assert!(
            result.is_ok(),
            "Fallback request {} should succeed",
            i + 1
        );
    }

    // 11th fallback request should be rate limited
    let result = limiter.check_fallback_limit("tenant-1", "/api/invoices");
    assert!(
        result.is_err(),
        "Fallback should be rate limited after 10 requests"
    );

    // But normal reads should still work (separate quota)
    let result = limiter.check_limit("tenant-1", "/api/invoices");
    assert!(
        result.is_ok(),
        "Normal reads should still work (separate quota from fallback)"
    );

    println!("✓ Fallback limits (10/min) are 10x tighter than normal limits (100/min)");
}

#[tokio::test]
async fn test_rate_limit_engages_under_load() {
    // Create rate limiter with very low quota for testing
    let limiter = RateLimiter::with_configs(
        RateLimitConfig::new(3, Duration::from_secs(60)),  // Only 3 requests/min
        RateLimitConfig::new(1, Duration::from_secs(60)),  // Only 1 fallback/min
    );

    let tenant_id = "tenant-load-test";
    let path = "/api/invoices";

    // Simulate load: send 10 requests
    let mut success_count = 0;
    let mut rate_limited_count = 0;

    for _ in 0..10 {
        match limiter.check_limit(tenant_id, path) {
            Ok(_) => success_count += 1,
            Err(_) => rate_limited_count += 1,
        }
    }

    // First 3 should succeed, rest should be rate limited
    assert_eq!(success_count, 3, "Should allow exactly 3 requests");
    assert_eq!(rate_limited_count, 7, "Should rate limit 7 requests");

    println!("✓ Rate limit engaged: {}/{} requests allowed, {}/{} rate limited",
        success_count, 10, rate_limited_count, 10);
}

#[tokio::test]
async fn test_system_remains_healthy_under_rate_limiting() {
    // This test verifies that rate limiting doesn't crash the system
    // and that subsequent requests can still succeed

    let limiter = Arc::new(RateLimiter::with_configs(
        RateLimitConfig::new(5, Duration::from_secs(60)),
        RateLimitConfig::new(2, Duration::from_secs(60)),
    ));

    let tenant_id = "tenant-health-test";

    // Exhaust quota
    for _ in 0..5 {
        limiter.check_limit(tenant_id, "/api/invoices").ok();
    }

    // Send many more requests (should all be rate limited but not crash)
    for _ in 0..100 {
        let result = limiter.check_limit(tenant_id, "/api/invoices");
        assert!(result.is_err(), "All requests should be rate limited");
    }

    // System should still be responsive
    let quota = limiter.remaining_quota(tenant_id, "/api/invoices");
    assert_eq!(quota, 0, "Quota should be exhausted but system still responsive");

    // Different tenant should still work
    let result = limiter.check_limit("different-tenant", "/api/invoices");
    assert!(result.is_ok(), "Different tenant should not be affected");

    println!("✓ System remained healthy: 100 rate-limited requests didn't crash");
}

#[tokio::test]
async fn test_quota_refills_over_time() {
    // Create limiter with fast refill for testing
    let limiter = RateLimiter::with_configs(
        RateLimitConfig::new(2, Duration::from_millis(200)),  // 2 per 200ms = 10/sec
        RateLimitConfig::new(1, Duration::from_millis(200)),  // 1 per 200ms = 5/sec
    );

    let tenant_id = "tenant-refill-test";
    let path = "/api/invoices";

    // Exhaust quota (2 requests)
    assert!(limiter.check_limit(tenant_id, path).is_ok());
    assert!(limiter.check_limit(tenant_id, path).is_ok());
    assert!(limiter.check_limit(tenant_id, path).is_err(), "Should be rate limited");

    // Wait for token refill
    tokio::time::sleep(Duration::from_millis(250)).await;

    // Should have refilled ~2.5 tokens, so 2 requests should work
    assert!(
        limiter.check_limit(tenant_id, path).is_ok(),
        "Should succeed after refill"
    );
    assert!(
        limiter.check_limit(tenant_id, path).is_ok(),
        "Should succeed after refill"
    );

    println!("✓ Token bucket refilled over time as expected");
}

#[tokio::test]
async fn test_different_endpoints_have_separate_quotas() {
    let limiter = RateLimiter::with_configs(
        RateLimitConfig::new(3, Duration::from_secs(60)),
        RateLimitConfig::new(1, Duration::from_secs(60)),
    );

    let tenant_id = "tenant-endpoint-test";

    // Exhaust quota for /api/invoices
    for _ in 0..3 {
        assert!(limiter.check_limit(tenant_id, "/api/invoices").is_ok());
    }
    assert!(limiter.check_limit(tenant_id, "/api/invoices").is_err());

    // /api/payments should have separate quota
    assert!(
        limiter.check_limit(tenant_id, "/api/payments").is_ok(),
        "Different endpoint should have separate quota"
    );

    println!("✓ Different endpoints have independent rate limit quotas");
}

#[test]
fn test_rate_limit_config_defaults() {
    let normal = RateLimitConfig::normal_read();
    assert_eq!(normal.max_requests, 100);
    assert_eq!(normal.window, Duration::from_secs(60));

    let fallback = RateLimitConfig::fallback();
    assert_eq!(fallback.max_requests, 10);
    assert_eq!(fallback.window, Duration::from_secs(60));

    // Fallback should be 10x tighter
    assert_eq!(
        normal.max_requests / fallback.max_requests,
        10,
        "Fallback should be 10x tighter than normal"
    );

    println!("✓ Rate limit config defaults verified");
}
