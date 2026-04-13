//! Configuration for the reconciliation runner.
//!
//! Each module has its own database. All DATABASE_URL env vars are required
//! unless `--skip-missing` is passed, in which case modules with no DB URL
//! are silently omitted from the run.

use std::env;

/// Per-module database connection URL. None means "skip this module".
#[derive(Debug, Clone)]
pub struct Config {
    pub ar_database_url: Option<String>,
    pub ap_database_url: Option<String>,
    pub gl_database_url: Option<String>,
    pub inventory_database_url: Option<String>,
    pub bom_database_url: Option<String>,
    pub production_database_url: Option<String>,
    /// Directory to write Prometheus textfile metrics into.
    /// Default: /var/lib/prometheus/node_exporter
    /// Set to "-" to write to stdout.
    pub metrics_output: String,
}

impl Config {
    /// Load from environment variables. Missing vars → None (module skipped).
    pub fn from_env() -> Self {
        Self {
            ar_database_url: env_opt("AR_DATABASE_URL"),
            ap_database_url: env_opt("AP_DATABASE_URL"),
            gl_database_url: env_opt("GL_DATABASE_URL"),
            inventory_database_url: env_opt("INVENTORY_DATABASE_URL"),
            bom_database_url: env_opt("BOM_DATABASE_URL"),
            production_database_url: env_opt("PRODUCTION_DATABASE_URL"),
            metrics_output: env::var("RECON_METRICS_OUTPUT")
                .unwrap_or_else(|_| "/var/lib/prometheus/node_exporter".to_string()),
        }
    }

    /// Returns true if at least one module DB URL is configured.
    pub fn has_any_module(&self) -> bool {
        self.ar_database_url.is_some()
            || self.ap_database_url.is_some()
            || self.gl_database_url.is_some()
            || self.inventory_database_url.is_some()
            || self.bom_database_url.is_some()
            || self.production_database_url.is_some()
    }
}

fn env_opt(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}
