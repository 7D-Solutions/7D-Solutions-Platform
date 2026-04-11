//! Composite key isolation tests for TieredRateLimiter.
//!
//! Acceptance criteria (from bd-6sle9):
//! 1. Two requests from the same IP but different tenant_ids are rate-limited independently.
//! 2. Two requests from different IPs but the same tenant_id are rate-limited independently.
//!
//! These tests run against the real in-process rate limiter — no mocks, no stubs.

use security::ratelimit::{RateLimitConfig, RateLimitKeyStrategy, TieredRateLimiter};
use std::time::Duration;

fn build_composite_limiter(limit: u32) -> TieredRateLimiter {
    TieredRateLimiter::with_strategies(vec![(
        "api".to_string(),
        RateLimitConfig::new(limit, Duration::from_secs(60)),
        vec!["/api/".to_string()],
        RateLimitKeyStrategy::Composite,
    )])
}

fn build_ip_only_limiter(limit: u32) -> TieredRateLimiter {
    TieredRateLimiter::with_strategies(vec![(
        "api".to_string(),
        RateLimitConfig::new(limit, Duration::from_secs(60)),
        vec!["/api/".to_string()],
        RateLimitKeyStrategy::IpOnly,
    )])
}

fn build_tenant_only_limiter(limit: u32) -> TieredRateLimiter {
    TieredRateLimiter::with_strategies(vec![(
        "api".to_string(),
        RateLimitConfig::new(limit, Duration::from_secs(60)),
        vec!["/api/".to_string()],
        RateLimitKeyStrategy::TenantOnly,
    )])
}

// ── Composite: same IP, different tenants ─────────────────────────────────────

/// Same IP, different tenant_ids → independent buckets.
///
/// This is the primary acceptance criterion: one tenant's traffic must not
/// exhaust another tenant's rate limit even when both come from the same IP.
#[test]
fn composite_same_ip_different_tenants_are_independent() {
    let limiter = build_composite_limiter(3);

    // Exhaust tenant_a's quota (same IP as tenant_b).
    for _ in 0..3 {
        assert!(
            limiter
                .check_limit("/api/orders", "tenant_a", "10.0.0.1")
                .is_ok(),
            "tenant_a within quota should be allowed"
        );
    }
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.1")
            .is_err(),
        "tenant_a over quota should be blocked"
    );

    // tenant_b on the SAME IP must still have its full quota.
    for _ in 0..3 {
        assert!(
            limiter
                .check_limit("/api/orders", "tenant_b", "10.0.0.1")
                .is_ok(),
            "tenant_b must be unaffected by tenant_a's exhaustion"
        );
    }
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_b", "10.0.0.1")
            .is_err(),
        "tenant_b over quota should be blocked"
    );
}

// ── Composite: same tenant, different IPs ─────────────────────────────────────

/// Same tenant_id, different IPs → independent buckets.
///
/// A tenant connecting from multiple IPs (e.g. distributed clients or load
/// balancers) must have separate buckets per IP so that one IP's traffic
/// cannot exhaust the quota for another IP.
#[test]
fn composite_same_tenant_different_ips_are_independent() {
    let limiter = build_composite_limiter(3);

    // Exhaust tenant_a from IP 10.0.0.1.
    for _ in 0..3 {
        assert!(
            limiter
                .check_limit("/api/orders", "tenant_a", "10.0.0.1")
                .is_ok(),
            "tenant_a from ip1 within quota"
        );
    }
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.1")
            .is_err(),
        "tenant_a from ip1 over quota"
    );

    // The SAME tenant from a DIFFERENT IP must have its own independent bucket.
    for _ in 0..3 {
        assert!(
            limiter
                .check_limit("/api/orders", "tenant_a", "10.0.0.2")
                .is_ok(),
            "tenant_a from ip2 must be unaffected by ip1 exhaustion"
        );
    }
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.2")
            .is_err(),
        "tenant_a from ip2 over quota"
    );
}

// ── IpOnly: tenants share bucket per IP ───────────────────────────────────────

/// IpOnly strategy: different tenant_ids on the same IP share one bucket.
#[test]
fn ip_only_tenants_share_bucket_per_ip() {
    let limiter = build_ip_only_limiter(4);

    // tenant_a consumes 2 tokens.
    assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.1").is_ok());
    assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.1").is_ok());

    // tenant_b on the same IP shares the same bucket — only 2 left.
    assert!(limiter.check_limit("/api/orders", "tenant_b", "10.0.0.1").is_ok());
    assert!(limiter.check_limit("/api/orders", "tenant_b", "10.0.0.1").is_ok());

    // Bucket exhausted — both tenants should be blocked on that IP.
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.1")
            .is_err(),
        "IpOnly: tenant_a should be blocked once shared bucket exhausted"
    );
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_b", "10.0.0.1")
            .is_err(),
        "IpOnly: tenant_b should also be blocked from the shared bucket"
    );

    // Different IP has a fresh bucket.
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.2")
            .is_ok(),
        "IpOnly: different IP has its own bucket"
    );
}

// ── TenantOnly: IPs share bucket per tenant ───────────────────────────────────

/// TenantOnly strategy: different IPs for the same tenant share one bucket.
#[test]
fn tenant_only_ips_share_bucket_per_tenant() {
    let limiter = build_tenant_only_limiter(4);

    // tenant_a from ip1 consumes 2 tokens.
    assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.1").is_ok());
    assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.1").is_ok());

    // tenant_a from ip2 shares the same per-tenant bucket — only 2 left.
    assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.2").is_ok());
    assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.2").is_ok());

    // Bucket exhausted — both IPs should be blocked.
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.1")
            .is_err(),
        "TenantOnly: ip1 should be blocked once tenant bucket exhausted"
    );
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.2")
            .is_err(),
        "TenantOnly: ip2 should also be blocked from the shared tenant bucket"
    );

    // Different tenant has a fresh bucket.
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_b", "10.0.0.1")
            .is_ok(),
        "TenantOnly: different tenant has its own bucket"
    );
}

// ── Mixed tiers: per-tier strategy ────────────────────────────────────────────

/// Different tiers in the same limiter can have different key strategies.
#[test]
fn mixed_tier_strategies_are_independently_enforced() {
    // "login" tier: IP-only (public, unauthenticated)
    // "api" tier:   Composite (authenticated multi-tenant)
    let limiter = TieredRateLimiter::with_strategies(vec![
        (
            "login".to_string(),
            RateLimitConfig::new(2, Duration::from_secs(60)),
            vec!["/api/auth/".to_string()],
            RateLimitKeyStrategy::IpOnly,
        ),
        (
            "api".to_string(),
            RateLimitConfig::new(5, Duration::from_secs(60)),
            vec!["/api/".to_string()],
            RateLimitKeyStrategy::Composite,
        ),
    ]);

    // Login tier (IpOnly): exhaust ip1 across both tenants.
    assert!(limiter.check_limit("/api/auth/token", "tenant_a", "10.0.0.1").is_ok());
    assert!(limiter.check_limit("/api/auth/token", "tenant_b", "10.0.0.1").is_ok());
    // Both tokens consumed for ip1 — both tenants blocked.
    assert!(
        limiter
            .check_limit("/api/auth/token", "tenant_a", "10.0.0.1")
            .is_err(),
        "login/IpOnly: tenant_a blocked after shared ip1 bucket exhausted"
    );

    // API tier (Composite): tenant_a and tenant_b have independent buckets on ip1.
    for _ in 0..5 {
        assert!(limiter.check_limit("/api/orders", "tenant_a", "10.0.0.1").is_ok());
    }
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.1")
            .is_err(),
        "api/Composite: tenant_a exhausted"
    );
    // tenant_b on the same IP is unaffected.
    assert!(
        limiter
            .check_limit("/api/orders", "tenant_b", "10.0.0.1")
            .is_ok(),
        "api/Composite: tenant_b must be independent of tenant_a"
    );
}

// ── Remaining quota respects strategy ────────────────────────────────────────

#[test]
fn remaining_quota_uses_tier_strategy() {
    let limiter = build_composite_limiter(10);

    // Consume 3 tokens for (tenant_a, ip1).
    for _ in 0..3 {
        limiter
            .check_limit("/api/orders", "tenant_a", "10.0.0.1")
            .expect("within quota");
    }

    // tenant_a from ip1: should show 7 remaining.
    assert_eq!(
        limiter.remaining_quota("/api/orders", "tenant_a", "10.0.0.1"),
        7,
        "remaining quota decreases after consumption"
    );

    // tenant_b from ip1: full quota unaffected.
    assert_eq!(
        limiter.remaining_quota("/api/orders", "tenant_b", "10.0.0.1"),
        10,
        "different tenant on same IP has independent quota"
    );

    // tenant_a from ip2: full quota unaffected.
    assert_eq!(
        limiter.remaining_quota("/api/orders", "tenant_a", "10.0.0.2"),
        10,
        "same tenant on different IP has independent quota"
    );
}
