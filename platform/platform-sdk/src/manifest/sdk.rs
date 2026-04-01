use std::collections::BTreeMap;

use serde::Deserialize;

/// `[sdk]` — SDK compatibility constraints.
#[derive(Debug, Clone, Deserialize)]
pub struct SdkSection {
    #[serde(default)]
    pub min_version: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
