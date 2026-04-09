use std::collections::BTreeMap;

use serde::Deserialize;

/// Per-tier rate limit configuration for `[rate_limit.tiers.<name>]`.
#[derive(Debug, Clone, Deserialize)]
pub struct TierSection {
    /// Maximum requests allowed in the time window.
    pub requests_per_window: u32,
    /// Window duration in seconds (default: 60).
    #[serde(default = "default_window_seconds")]
    pub window_seconds: u64,
    /// Path prefixes assigned to this tier.
    #[serde(default)]
    pub routes: Vec<String>,
}

fn default_window_seconds() -> u64 {
    60
}

/// `[rate_limit]` — request rate limiting.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitSection {
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,

    /// Named rate limit tiers. Configured as `[rate_limit.tiers.<name>]` in TOML.
    ///
    /// Example:
    /// ```toml
    /// [rate_limit.tiers.api]
    /// requests_per_window = 1000
    /// window_seconds = 60
    /// routes = ["/api/"]
    ///
    /// [rate_limit.tiers.login]
    /// requests_per_window = 10
    /// window_seconds = 60
    /// routes = ["/api/auth/", "/api/login"]
    /// ```
    #[serde(default)]
    pub tiers: BTreeMap<String, TierSection>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for RateLimitSection {
    fn default() -> Self {
        Self {
            requests_per_second: default_rps(),
            burst: default_burst(),
            tiers: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

fn default_rps() -> u32 {
    100
}

fn default_burst() -> u32 {
    200
}
