//! `[platform.services]` — declare which platform services this module calls.
//!
//! At startup the SDK reads each entry, resolves the base URL from an env var
//! (e.g. `PARTY_BASE_URL`), builds a `PlatformClient`, and stores it so
//! handlers can retrieve typed clients via `ctx.platform_client::<T>()`.
//!
//! ## Criticality
//!
//! Each service may declare a `criticality` field:
//!
//! ```toml
//! [platform.services]
//! numbering     = { enabled = true, criticality = "critical" }
//! notifications = { enabled = true, criticality = "degraded", default_url = "http://7d-notifications:8089" }
//! audit-log     = { enabled = true, criticality = "best-effort" }
//! ```
//!
//! - `critical` (default) — startup fails if URL is unresolvable; callers use
//!   `ctx.platform_client::<T>()` or `ctx.critical_client::<T>()`.
//! - `degraded` — startup **succeeds** even when the URL is absent; callers use
//!   `ctx.degraded_client::<T>()` which returns `Err(DegradedMode::Unavailable)`
//!   so handlers can succeed with an `X-Degraded` warning header.
//! - `best-effort` — same startup semantics as `degraded`; fire-and-forget
//!   callers that never block on the result.

use std::collections::BTreeMap;

use serde::Deserialize;

/// How the module should behave when this service dependency is unavailable.
///
/// See the module-level documentation for the `[platform.services]` semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceCriticality {
    /// Startup fails if the URL cannot be resolved; callers expect the service
    /// to always be reachable.  This is the default for existing deps.
    #[default]
    Critical,

    /// Startup continues when the URL is absent.  `ctx.degraded_client::<T>()`
    /// returns `Err(DegradedMode::Unavailable)` so the caller can degrade
    /// gracefully (e.g. return `X-Degraded` header) instead of failing.
    Degraded,

    /// Same startup semantics as `degraded`.  Intended for fire-and-forget
    /// side effects (metrics, audit logs) where the caller never blocks.
    BestEffort,
}

impl ServiceCriticality {
    /// Returns `true` for `Degraded` or `BestEffort` — i.e. non-critical services
    /// whose URL may be absent at startup without causing a failure.
    pub fn is_non_critical(self) -> bool {
        matches!(self, Self::Degraded | Self::BestEffort)
    }
}

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
    /// If omitted and the env var is missing, startup fails with a clear error
    /// for `critical` services; for `degraded`/`best-effort` services the module
    /// starts without a client and `ctx.degraded_client` returns `Unavailable`.
    #[serde(default)]
    pub default_url: Option<String>,

    /// How to handle unavailability of this service. Default: `critical`.
    ///
    /// See [`ServiceCriticality`] for the full semantics.
    #[serde(default)]
    pub criticality: ServiceCriticality,

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
        format!("{}_BASE_URL", service_name.to_uppercase().replace('-', "_"))
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

    #[test]
    fn criticality_defaults_to_critical() {
        let toml_str = r#"
[services]
party = { enabled = true }
"#;
        let section: PlatformSection = toml::from_str(toml_str).expect("parse");
        assert_eq!(
            section.services["party"].criticality,
            ServiceCriticality::Critical
        );
    }

    #[test]
    fn criticality_degraded_parses() {
        let toml_str = r#"
[services]
notifications = { enabled = true, criticality = "degraded" }
"#;
        let section: PlatformSection = toml::from_str(toml_str).expect("parse");
        assert_eq!(
            section.services["notifications"].criticality,
            ServiceCriticality::Degraded
        );
        assert!(section.services["notifications"]
            .criticality
            .is_non_critical());
    }

    #[test]
    fn criticality_best_effort_parses() {
        let toml_str = r#"
[services]
audit-log = { enabled = true, criticality = "best-effort" }
"#;
        let section: PlatformSection = toml::from_str(toml_str).expect("parse");
        assert_eq!(
            section.services["audit-log"].criticality,
            ServiceCriticality::BestEffort
        );
        assert!(section.services["audit-log"].criticality.is_non_critical());
    }

    #[test]
    fn criticality_critical_explicit_parses() {
        let toml_str = r#"
[services]
numbering = { enabled = true, criticality = "critical" }
"#;
        let section: PlatformSection = toml::from_str(toml_str).expect("parse");
        assert_eq!(
            section.services["numbering"].criticality,
            ServiceCriticality::Critical
        );
        assert!(!section.services["numbering"].criticality.is_non_critical());
    }
}
