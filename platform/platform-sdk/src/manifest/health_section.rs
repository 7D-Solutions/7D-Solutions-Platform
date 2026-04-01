use std::collections::BTreeMap;

use serde::Deserialize;

/// `[health]` — health-check dependency probing.
#[derive(Debug, Clone, Deserialize)]
pub struct HealthSection {
    #[serde(default)]
    pub dependencies: Vec<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Default for HealthSection {
    fn default() -> Self {
        Self {
            dependencies: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// Known dependency types for health probing.
pub const KNOWN_HEALTH_DEPS: &[&str] = &["postgres", "nats"];
