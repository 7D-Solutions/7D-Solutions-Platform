//! Configuration for the stabilization gate harness.
//!
//! All settings are read from environment variables with safe defaults.
//! DATABASE_URL (or AR_DATABASE_URL) and NATS_URL are required for real runs.

use std::env;

/// Runtime configuration read from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Number of tenants to simulate (TENANT_COUNT, default 5)
    pub tenant_count: usize,
    /// Events to publish per tenant (EVENTS_PER_TENANT, default 100)
    pub events_per_tenant: usize,
    /// Rows to process in reconciliation benchmark (RECON_ROWS, default 500)
    pub recon_rows: usize,
    /// Rows to process in dunning benchmark (DUNNING_ROWS, default 200)
    pub dunning_rows: usize,
    /// Worker concurrency (CONCURRENCY, default 4)
    pub concurrency: usize,
    /// Duration for timed benchmarks in seconds (DURATION_SECS, default 30)
    pub duration_secs: u64,
    /// PostgreSQL connection URL (DATABASE_URL or AR_DATABASE_URL)
    pub database_url: String,
    /// NATS server URL (NATS_URL, default nats://localhost:4222)
    pub nats_url: String,
}

impl Config {
    /// Load configuration from environment. Fails fast if required vars are missing.
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL")
            .or_else(|_| env::var("AR_DATABASE_URL"))
            .map_err(|_| anyhow::anyhow!(
                "Missing DATABASE_URL or AR_DATABASE_URL. \
                 Example: DATABASE_URL=postgres://postgres:postgres@localhost:5432/ar_db"
            ))?;

        let nats_url = env::var("NATS_URL")
            .unwrap_or_else(|_| "nats://localhost:4222".to_string());

        Ok(Self {
            tenant_count: parse_env("TENANT_COUNT", 5)?,
            events_per_tenant: parse_env("EVENTS_PER_TENANT", 100)?,
            recon_rows: parse_env("RECON_ROWS", 500)?,
            dunning_rows: parse_env("DUNNING_ROWS", 200)?,
            concurrency: parse_env("CONCURRENCY", 4)?,
            duration_secs: parse_env("DURATION_SECS", 30)?,
            database_url,
            nats_url,
        })
    }

    /// Produce a safe env snapshot for report embedding (no credentials).
    pub fn env_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "tenant_count": self.tenant_count,
            "events_per_tenant": self.events_per_tenant,
            "recon_rows": self.recon_rows,
            "dunning_rows": self.dunning_rows,
            "concurrency": self.concurrency,
            "duration_secs": self.duration_secs,
            "nats_url": self.nats_url,
            "database_host": extract_host(&self.database_url),
        })
    }
}

fn parse_env<T>(key: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(val) => val.parse::<T>().map_err(|e| {
            anyhow::anyhow!("Invalid value for env var {}: {}", key, e)
        }),
        Err(_) => Ok(default),
    }
}

/// Extract only the host:port from a Postgres URL for safe logging.
fn extract_host(url: &str) -> String {
    // postgres://user:pass@host:port/db  →  host:port
    url.split('@')
        .next_back()
        .and_then(|s| s.split('/').next())
        .unwrap_or("unknown")
        .to_string()
}
