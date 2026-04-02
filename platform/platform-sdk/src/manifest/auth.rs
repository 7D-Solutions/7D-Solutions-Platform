use std::collections::BTreeMap;

use serde::Deserialize;

/// `[auth]` — JWT / JWKS authentication configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthSection {
    #[serde(default)]
    pub jwks_url: Option<String>,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: String,
    #[serde(default = "default_true")]
    pub fallback_to_env: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When `true` (the default), startup fails if no JWT verifier can be
    /// created (JWKS unreachable and no `JWT_PUBLIC_KEY` env var).  Modules
    /// that intentionally run without authentication must set this to `false`.
    #[serde(default = "default_true")]
    pub required: bool,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for AuthSection {
    fn default() -> Self {
        Self {
            jwks_url: None,
            refresh_interval: default_refresh_interval(),
            fallback_to_env: true,
            enabled: true,
            required: true,
            extra: BTreeMap::new(),
        }
    }
}

fn default_refresh_interval() -> String {
    "5m".to_string()
}

fn default_true() -> bool {
    true
}
