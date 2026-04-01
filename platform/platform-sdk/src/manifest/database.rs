use std::collections::BTreeMap;

use serde::Deserialize;

/// `[database]` — migration path and auto-migrate toggle.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    pub migrations: String,
    #[serde(default)]
    pub auto_migrate: bool,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
