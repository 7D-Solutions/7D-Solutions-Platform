use std::collections::BTreeMap;

use serde::Deserialize;

/// `[database]` — migration path, auto-migrate toggle, and pool sizing.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    pub migrations: String,
    #[serde(default)]
    pub auto_migrate: bool,
    #[serde(default = "default_pool_min")]
    pub pool_min: u32,
    #[serde(default = "default_pool_max")]
    pub pool_max: u32,
    /// Maximum seconds to wait for a free connection before failing.
    /// Defaults to 5. Set higher if your module runs long-lived transactions.
    #[serde(default = "default_pool_acquire_timeout_secs")]
    pub pool_acquire_timeout_secs: u64,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

fn default_pool_min() -> u32 {
    5
}

fn default_pool_max() -> u32 {
    20
}

fn default_pool_acquire_timeout_secs() -> u64 {
    5
}
