use std::collections::BTreeMap;

use serde::Deserialize;

/// `[rate_limit]` — request rate limiting.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitSection {
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for RateLimitSection {
    fn default() -> Self {
        Self {
            requests_per_second: default_rps(),
            burst: default_burst(),
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
