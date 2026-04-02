//! `[platform.services]` — declare which platform services this module calls.
//!
//! At startup the SDK reads each entry, resolves the base URL from an env var
//! (e.g. `PARTY_BASE_URL`), builds a `PlatformClient`, and stores it so
//! handlers can retrieve typed clients via `ctx.platform_client::<T>()`.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top-level `[platform]` section in module.toml.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlatformSection {
    /// Map of service-name → config.
    ///
    /// ```toml
    /// [platform.services]
    /// party     = { enabled = true }
    /// inventory = { enabled = true, timeout_secs = 60 }
    /// bom       = { enabled = true, default_url = "http://localhost:8107" }
    /// ```
    #[serde(default)]
    pub services: BTreeMap<String, ServiceEntry>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// Per-service configuration entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceEntry {
    /// Whether this service is active (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Per-service request timeout in seconds (overrides PlatformClient default).
    #[serde(default)]
    pub timeout_secs: Option<u64>,

    /// Fallback base URL when the env var is not set.
    ///
    /// If omitted and the env var is missing, startup fails with a clear error.
    #[serde(default)]
    pub default_url: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

fn default_true() -> bool {
    true
}

impl ServiceEntry {
    /// Derive the env var name for this service's base URL.
    ///
    /// `party` → `PARTY_BASE_URL`, `shipping-receiving` → `SHIPPING_RECEIVING_BASE_URL`.
    pub fn env_var_name(service_name: &str) -> String {
        format!(
            "{}_BASE_URL",
            service_name.to_uppercase().replace('-', "_")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_name_simple() {
        assert_eq!(ServiceEntry::env_var_name("party"), "PARTY_BASE_URL");
    }

    #[test]
    fn env_var_name_hyphenated() {
        assert_eq!(
            ServiceEntry::env_var_name("shipping-receiving"),
            "SHIPPING_RECEIVING_BASE_URL"
        );
    }

    #[test]
    fn parse_services_section() {
        let toml_str = r#"
[services]
party = { enabled = true }
inventory = { enabled = true, timeout_secs = 60 }
bom = { enabled = true, default_url = "http://localhost:8107" }
"#;
        let section: PlatformSection = toml::from_str(toml_str).expect("parse");
        assert_eq!(section.services.len(), 3);
        assert!(section.services["party"].enabled);
        assert_eq!(section.services["inventory"].timeout_secs, Some(60));
        assert_eq!(
            section.services["bom"].default_url.as_deref(),
            Some("http://localhost:8107")
        );
    }

    #[test]
    fn disabled_service() {
        let toml_str = r#"
[services]
party = { enabled = false }
"#;
        let section: PlatformSection = toml::from_str(toml_str).expect("parse");
        assert!(!section.services["party"].enabled);
    }

    #[test]
    fn empty_services_defaults() {
        let section: PlatformSection = toml::from_str("").expect("parse empty");
        assert!(section.services.is_empty());
    }
}
