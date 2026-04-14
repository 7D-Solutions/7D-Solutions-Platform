use std::collections::BTreeMap;

use serde::Deserialize;

/// `[bus]` — event bus configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct BusSection {
    #[serde(rename = "type", default = "default_bus_type")]
    pub bus_type: String,

    /// Event stream class for this module.
    ///
    /// Declares which dedup-window class the module's events belong to.
    /// Used for documentation and validation; the actual stream configuration
    /// is applied platform-wide at startup via `ensure_platform_streams`.
    ///
    /// Valid values: `"financial"`, `"operational"`, `"notification"`, `"system"`.
    ///
    /// Examples:
    /// - `ap`, `ar`, `gl`, `payments` → `"financial"`
    /// - `production`, `inventory`, `shipping` → `"operational"`
    /// - `notifications` → `"notification"`
    /// - `auth`, `tenant-registry` → `"system"`
    #[serde(default)]
    pub stream_class: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

fn default_bus_type() -> String {
    "inmemory".to_string()
}
