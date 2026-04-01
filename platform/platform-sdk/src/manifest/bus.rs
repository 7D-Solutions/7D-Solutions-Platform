use std::collections::BTreeMap;

use serde::Deserialize;

/// `[bus]` — event bus configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct BusSection {
    #[serde(rename = "type", default = "default_bus_type")]
    pub bus_type: String,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

fn default_bus_type() -> String {
    "inmemory".to_string()
}
