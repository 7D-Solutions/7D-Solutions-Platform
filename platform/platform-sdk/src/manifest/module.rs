use std::collections::BTreeMap;

use serde::Deserialize;

/// `[module]` — identity and metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleSection {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
