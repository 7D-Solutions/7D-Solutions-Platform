use std::collections::BTreeMap;

use serde::Deserialize;

/// `[database]` — migration path, auto-migrate toggle, and pool sizing.
///
/// ## Pool workload classes
///
/// Set these fields under `[database]` in `module.toml` to match your module's workload:
///
/// | Class        | pool_min | pool_max | Modules                                |
/// |--------------|----------|----------|----------------------------------------|
/// | write-heavy  | 2        | 20–30    | ap(20), ar(25), gl(15), payments(30), production(20) |
/// | read-heavy   | 2        | 10–15    | inventory(15), bom(10), shipping-receiving(15) |
/// | mixed        | 2        | 12       | all other modules                       |
///
/// All classes: `pool_acquire_timeout_secs = 5`, `pool_idle_timeout_secs = 300`.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    pub migrations: String,
    #[serde(default)]
    pub auto_migrate: bool,
    /// Minimum idle connections kept alive. Default: 2.
    #[serde(default = "default_pool_min")]
    pub pool_min: u32,
    /// Maximum concurrent connections. Default: 12 (mixed class).
    /// Tune per workload class — see table above.
    #[serde(default = "default_pool_max")]
    pub pool_max: u32,
    /// Maximum seconds to wait for a free connection before failing.
    /// Default: 5. Increase for modules with long-lived transactions.
    #[serde(default = "default_pool_acquire_timeout_secs")]
    pub pool_acquire_timeout_secs: u64,
    /// Seconds a connection may remain idle before being closed and removed
    /// from the pool. Default: 300 (5 minutes).
    #[serde(default = "default_pool_idle_timeout_secs")]
    pub pool_idle_timeout_secs: u64,
    /// Per-tenant connection budget. Default: 5 concurrent connections.
    #[serde(default)]
    pub tenant_quota: Option<TenantQuotaSection>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// `[database.tenant_quota]` — in-memory per-tenant connection budget.
#[derive(Debug, Clone, Deserialize)]
pub struct TenantQuotaSection {
    /// Maximum concurrent DB connections allowed per tenant.
    #[serde(default = "default_tenant_quota_max_connections")]
    pub max_connections: u32,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

fn default_pool_min() -> u32 {
    2
}

fn default_pool_max() -> u32 {
    12
}

fn default_pool_acquire_timeout_secs() -> u64 {
    5
}

fn default_pool_idle_timeout_secs() -> u64 {
    300
}

fn default_tenant_quota_max_connections() -> u32 {
    5
}
