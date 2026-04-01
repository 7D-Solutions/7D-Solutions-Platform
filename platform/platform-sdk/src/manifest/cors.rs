use std::collections::BTreeMap;

use serde::Deserialize;

/// `[cors]` — CORS origin configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct CorsSection {
    #[serde(default)]
    pub origins: Option<Vec<String>>,
    #[serde(default)]
    pub origin_pattern: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for CorsSection {
    fn default() -> Self {
        Self {
            origins: None,
            origin_pattern: None,
            extra: BTreeMap::new(),
        }
    }
}
