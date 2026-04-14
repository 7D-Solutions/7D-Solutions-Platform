//! Parser for `module.toml` manifest files.
//!
//! Each platform module ships a `module.toml` at its crate root describing
//! its identity, database requirements, event bus preference, and SDK
//! compatibility range. The SDK reads this at startup to configure the
//! runtime without per-module boilerplate.
//!
//! # Example `module.toml`
//!
//! ```toml
//! [module]
//! name = "party"
//! version = "2.3.3"
//! description = "Party master data"
//!
//! [server]
//! host = "0.0.0.0"
//! port = 8098
//!
//! [database]
//! migrations = "./db/migrations"
//! auto_migrate = true
//!
//! [bus]
//! type = "inmemory"     # "nats" | "inmemory"
//!
//! [sdk]
//! min_version = "0.1.0"
//! ```

mod auth;
mod blob;
mod bus;
mod cors;
mod database;
mod events;
mod health_section;
mod module;
mod platform_services;
mod rate_limit;
mod sdk;
mod server;

pub use auth::AuthSection;
pub use blob::BlobSection;
pub use bus::BusSection;
pub use cors::CorsSection;
pub use database::{DatabaseSection, TenantQuotaSection};
pub use events::{EventsPublishSection, EventsSection};
pub use health_section::{HealthSection, KNOWN_HEALTH_DEPS};
pub use module::ModuleSection;
pub use platform_services::{PlatformSection, ServiceCriticality, ServiceEntry};
pub use rate_limit::RateLimitSection;
pub use sdk::SdkSection;
pub use server::ServerSection;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Errors that can occur while loading or validating a module manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse manifest TOML: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("manifest validation failed: {0}")]
    Validation(String),
}

/// Top-level structure of `module.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub module: ModuleSection,
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub database: Option<DatabaseSection>,
    #[serde(default)]
    pub bus: Option<BusSection>,
    #[serde(default)]
    pub events: Option<EventsSection>,
    #[serde(default)]
    pub sdk: Option<SdkSection>,
    #[serde(default)]
    pub auth: Option<AuthSection>,
    #[serde(default)]
    pub cors: Option<CorsSection>,
    #[serde(default)]
    pub health: Option<HealthSection>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitSection>,
    #[serde(default)]
    pub platform: Option<PlatformSection>,
    #[serde(default)]
    pub blob: Option<BlobSection>,

    /// Unknown top-level keys are captured here so we can warn without erroring.
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Manifest {
    /// Load and validate a manifest from the given file path.
    pub fn from_file(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_str(&content, Some(path))
    }

    /// Parse and validate a manifest from a TOML string.
    pub fn from_str(toml_str: &str, source_path: Option<&Path>) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(toml_str)?;
        manifest.validate(source_path)?;
        Ok(manifest)
    }

    fn validate(&self, source_path: Option<&Path>) -> Result<(), ManifestError> {
        // Module name must not be empty.
        if self.module.name.trim().is_empty() {
            return Err(ManifestError::Validation(
                "module.name must not be empty".into(),
            ));
        }

        // Validate bus type if present.
        if let Some(ref bus) = self.bus {
            let valid_types = ["nats", "inmemory", "none"];
            if !valid_types.contains(&bus.bus_type.to_lowercase().as_str()) {
                return Err(ManifestError::Validation(format!(
                    "bus.type must be one of {:?}, got '{}'",
                    valid_types, bus.bus_type
                )));
            }
        }

        // Validate events section if present.
        if let Some(ref events) = self.events {
            if let Some(ref publish) = events.publish {
                if publish.outbox_table.trim().is_empty() {
                    return Err(ManifestError::Validation(
                        "events.publish.outbox_table must not be empty".into(),
                    ));
                }
                if !publish
                    .outbox_table
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return Err(ManifestError::Validation(format!(
                        "events.publish.outbox_table '{}' contains invalid characters \
                         (only ASCII alphanumeric and underscores allowed)",
                        publish.outbox_table
                    )));
                }
                // Publishing requires a bus.
                let bus_type = self
                    .bus
                    .as_ref()
                    .map(|b| b.bus_type.to_lowercase())
                    .unwrap_or_default();
                if bus_type == "none" || bus_type.is_empty() {
                    return Err(ManifestError::Validation(
                        "events.publish.outbox_table is declared but no event bus is configured \
                         — set [bus] type to 'nats' or 'inmemory'"
                            .into(),
                    ));
                }
            }
        }

        // Validate migrations path exists if specified.
        if let Some(ref db) = self.database {
            if let Some(ref quota) = db.tenant_quota {
                if quota.max_connections == 0 {
                    return Err(ManifestError::Validation(
                        "database.tenant_quota.max_connections must be greater than zero".into(),
                    ));
                }
            }
            if let Some(base) = source_path.and_then(|p| p.parent()) {
                let migrations_path = base.join(&db.migrations);
                if !migrations_path.exists() {
                    return Err(ManifestError::Validation(format!(
                        "database.migrations path '{}' does not exist (resolved to '{}')",
                        db.migrations,
                        migrations_path.display()
                    )));
                }
            }
        }

        // Validate auth section.
        if let Some(ref auth) = self.auth {
            if auth.jwks_url.is_some() && !auth.enabled {
                return Err(ManifestError::Validation(
                    "auth.jwks_url is set but auth.enabled is false — \
                     either remove jwks_url or set enabled = true"
                        .into(),
                ));
            }
            if auth.required && !auth.enabled {
                return Err(ManifestError::Validation(
                    "auth.required is true but auth.enabled is false — \
                     either set required = false or enabled = true"
                        .into(),
                ));
            }
        }

        // Validate cors section.
        if let Some(ref cors) = self.cors {
            if cors.origins.is_some() && cors.origin_pattern.is_some() {
                return Err(ManifestError::Validation(
                    "cors.origins and cors.origin_pattern are mutually exclusive — pick one".into(),
                ));
            }
            if let Some(ref pattern) = cors.origin_pattern {
                regex::Regex::new(pattern).map_err(|e| {
                    ManifestError::Validation(format!(
                        "cors.origin_pattern '{}' is not a valid regex: {}",
                        pattern, e
                    ))
                })?;
            }
        }

        // Validate health section.
        if let Some(ref health) = self.health {
            for dep in &health.dependencies {
                if !KNOWN_HEALTH_DEPS.contains(&dep.as_str()) {
                    return Err(ManifestError::Validation(format!(
                        "health.dependencies contains unknown value '{}' — \
                         known values: {:?}",
                        dep, KNOWN_HEALTH_DEPS
                    )));
                }
            }
        }

        // SDK version compatibility check.
        if let Some(ref sdk) = self.sdk {
            if let Some(ref min_ver) = sdk.min_version {
                let required: semver::Version = min_ver.parse().map_err(|e| {
                    ManifestError::Validation(format!(
                        "sdk.min_version '{}' is not valid semver: {}",
                        min_ver, e
                    ))
                })?;

                let current: semver::Version = env!("CARGO_PKG_VERSION")
                    .parse()
                    .expect("CARGO_PKG_VERSION is always valid semver");
                if current < required {
                    return Err(ManifestError::Validation(format!(
                        "module requires platform-sdk >= {}, but this is {}",
                        required, current
                    )));
                }
            }
        }

        // Warn about unknown top-level keys.
        warn_extra_keys("", &self.extra);
        warn_extra_keys("module", &self.module.extra);
        warn_extra_keys("server", &self.server.extra);
        if let Some(ref db) = self.database {
            warn_extra_keys("database", &db.extra);
        }
        if let Some(ref bus) = self.bus {
            warn_extra_keys("bus", &bus.extra);
        }
        if let Some(ref events) = self.events {
            warn_extra_keys("events", &events.extra);
            if let Some(ref publish) = events.publish {
                warn_extra_keys("events.publish", &publish.extra);
            }
        }
        if let Some(ref sdk) = self.sdk {
            warn_extra_keys("sdk", &sdk.extra);
        }
        if let Some(ref auth) = self.auth {
            warn_extra_keys("auth", &auth.extra);
        }
        if let Some(ref cors) = self.cors {
            warn_extra_keys("cors", &cors.extra);
        }
        if let Some(ref health) = self.health {
            warn_extra_keys("health", &health.extra);
        }
        if let Some(ref rate_limit) = self.rate_limit {
            warn_extra_keys("rate_limit", &rate_limit.extra);
        }
        if let Some(ref platform) = self.platform {
            warn_extra_keys("platform", &platform.extra);
            for (name, entry) in &platform.services {
                warn_extra_keys(&format!("platform.services.{name}"), &entry.extra);
            }
        }
        if let Some(ref blob) = self.blob {
            warn_extra_keys("blob", &blob.extra);
        }

        Ok(())
    }
}

fn warn_extra_keys(section: &str, extra: &BTreeMap<String, toml::Value>) {
    for key in extra.keys() {
        let prefix = if section.is_empty() {
            String::new()
        } else {
            format!("{}.", section)
        };
        tracing::warn!("unknown key in module.toml: {}{}", prefix, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
[module]
name = "party"
version = "2.3.3"
description = "Party master data"

[server]
host = "0.0.0.0"
port = 8098

[bus]
type = "inmemory"

[sdk]
min_version = "0.1.0"
"#;

    #[test]
    fn valid_toml_parses() {
        let manifest = Manifest::from_str(VALID_TOML, None).expect("valid TOML should parse");
        assert_eq!(manifest.module.name, "party");
        assert_eq!(manifest.module.version.as_deref(), Some("2.3.3"));
        assert_eq!(manifest.server.port, 8098);
        assert_eq!(
            manifest
                .bus
                .as_ref()
                .expect("bus section")
                .bus_type
                .as_str(),
            "inmemory"
        );
    }

    #[test]
    fn minimal_toml_parses() {
        let toml_str = r#"
[module]
name = "minimal"
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("minimal TOML should parse");
        assert_eq!(manifest.module.name, "minimal");
        assert_eq!(manifest.server.host, "0.0.0.0");
        assert_eq!(manifest.server.port, 8080);
        assert!(manifest.database.is_none());
        assert!(manifest.bus.is_none());
    }

    #[test]
    fn empty_module_name_fails() {
        let toml_str = r#"
[module]
name = ""
"#;
        let err = Manifest::from_str(toml_str, None).expect_err("empty name should fail");
        assert!(
            matches!(err, ManifestError::Validation(_)),
            "expected validation error, got: {}",
            err
        );
    }

    #[test]
    fn invalid_bus_type_fails() {
        let toml_str = r#"
[module]
name = "test"

[bus]
type = "kafka"
"#;
        let err = Manifest::from_str(toml_str, None).expect_err("kafka should fail");
        match err {
            ManifestError::Validation(msg) => assert!(msg.contains("kafka")),
            other => panic!("expected validation error, got: {}", other),
        }
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let err =
            Manifest::from_str("not valid toml [[[", None).expect_err("invalid TOML should fail");
        assert!(matches!(err, ManifestError::Parse(_)));
    }

    #[test]
    fn sdk_version_compat_passes() {
        // min_version = "0.1.0" should pass since we ARE 0.1.0.
        let manifest = Manifest::from_str(VALID_TOML, None).expect("valid TOML should parse");
        assert!(manifest.sdk.is_some());
    }

    #[test]
    fn sdk_version_compat_fails_for_future() {
        let toml_str = r#"
[module]
name = "future"

[sdk]
min_version = "99.0.0"
"#;
        let err = Manifest::from_str(toml_str, None).expect_err("future version should fail");
        match err {
            ManifestError::Validation(msg) => assert!(msg.contains("99.0.0")),
            other => panic!("expected validation error, got: {}", other),
        }
    }

    #[test]
    fn invalid_semver_in_sdk_fails() {
        let toml_str = r#"
[module]
name = "bad-semver"

[sdk]
min_version = "not.a.version"
"#;
        let err = Manifest::from_str(toml_str, None).expect_err("bad semver should fail");
        match err {
            ManifestError::Validation(msg) => assert!(msg.contains("not valid semver")),
            other => panic!("expected validation error, got: {}", other),
        }
    }

    #[test]
    fn missing_migrations_path_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest_path = dir.path().join("module.toml");
        std::fs::write(
            &manifest_path,
            r#"
[module]
name = "test"

[database]
migrations = "./nonexistent/migrations"
auto_migrate = true
"#,
        )
        .expect("write manifest");

        let err = Manifest::from_file(&manifest_path).expect_err("missing migrations should fail");
        match err {
            ManifestError::Validation(msg) => assert!(msg.contains("does not exist")),
            other => panic!("expected validation error, got: {}", other),
        }
    }

    #[test]
    fn events_publish_section_parses() {
        let toml_str = r#"
[module]
name = "with-events"

[bus]
type = "nats"

[events.publish]
outbox_table = "events_outbox"
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("events section should parse");
        let publish = manifest
            .events
            .expect("events section")
            .publish
            .expect("publish section");
        assert_eq!(publish.outbox_table, "events_outbox");
    }

    #[test]
    fn empty_outbox_table_fails() {
        let toml_str = r#"
[module]
name = "bad-outbox"

[bus]
type = "nats"

[events.publish]
outbox_table = ""
"#;
        let err = Manifest::from_str(toml_str, None).expect_err("empty outbox table should fail");
        match err {
            ManifestError::Validation(msg) => assert!(msg.contains("must not be empty")),
            other => panic!("expected validation error, got: {}", other),
        }
    }

    #[test]
    fn outbox_table_without_bus_fails() {
        let toml_str = r#"
[module]
name = "no-bus"

[events.publish]
outbox_table = "events_outbox"
"#;
        let err = Manifest::from_str(toml_str, None).expect_err("outbox without bus should fail");
        match err {
            ManifestError::Validation(msg) => assert!(msg.contains("no event bus is configured")),
            other => panic!("expected validation error, got: {}", other),
        }
    }

    #[test]
    fn none_bus_type_parses() {
        let toml_str = r#"
[module]
name = "no-bus"

[bus]
type = "none"
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("none bus type should parse");
        assert_eq!(
            manifest
                .bus
                .as_ref()
                .expect("bus section")
                .bus_type
                .as_str(),
            "none"
        );
    }

    #[test]
    fn unknown_keys_dont_error() {
        let toml_str = r#"
[module]
name = "extras"
custom_field = "hello"

[unknown_section]
key = "value"
"#;
        // Should parse successfully — unknown keys warn but don't error.
        let manifest = Manifest::from_str(toml_str, None).expect("unknown keys should parse");
        assert_eq!(manifest.module.name, "extras");
        assert!(manifest.extra.contains_key("unknown_section"));
        assert!(manifest.module.extra.contains_key("custom_field"));
    }

    #[test]
    fn blob_section_parses() {
        let toml_str = r#"
[module]
name = "doc-mgmt"

[blob]
bucket = "platform-docs"
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("blob section should parse");
        let blob = manifest.blob.expect("blob section");
        assert_eq!(blob.bucket, "platform-docs");
    }

    #[test]
    fn no_blob_section_is_none() {
        let toml_str = r#"
[module]
name = "party"
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("manifest should parse");
        assert!(manifest.blob.is_none());
    }

    #[test]
    fn rate_limit_tiers_parse() {
        let toml_str = r#"
[module]
name = "with-tiers"

[rate_limit]
requests_per_second = 100
burst = 200

[rate_limit.tiers.api]
requests_per_window = 1000
window_seconds = 60
routes = ["/api/"]

[rate_limit.tiers.login]
requests_per_window = 10
window_seconds = 60
routes = ["/api/auth/", "/api/login"]
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("rate_limit tiers should parse");
        let rl = manifest.rate_limit.expect("rate_limit section");
        assert_eq!(rl.requests_per_second, 100);
        assert_eq!(rl.burst, 200);
        assert_eq!(rl.tiers.len(), 2);

        let api = rl.tiers.get("api").expect("api tier");
        assert_eq!(api.requests_per_window, 1000);
        assert_eq!(api.window_seconds, 60);
        assert_eq!(api.routes, vec!["/api/"]);

        let login = rl.tiers.get("login").expect("login tier");
        assert_eq!(login.requests_per_window, 10);
        assert_eq!(login.routes, vec!["/api/auth/", "/api/login"]);
    }

    #[test]
    fn rate_limit_tiers_default_window_seconds() {
        let toml_str = r#"
[module]
name = "tier-defaults"

[rate_limit.tiers.api]
requests_per_window = 500
routes = ["/api/"]
"#;
        let manifest =
            Manifest::from_str(toml_str, None).expect("tier with default window should parse");
        let rl = manifest.rate_limit.expect("rate_limit section");
        let api = rl.tiers.get("api").expect("api tier");
        assert_eq!(api.window_seconds, 60, "default window should be 60s");
    }

    #[test]
    fn rate_limit_tiers_empty_by_default() {
        let toml_str = r#"
[module]
name = "no-tiers"

[rate_limit]
requests_per_second = 50
burst = 100
"#;
        let manifest = Manifest::from_str(toml_str, None).expect("should parse without tiers");
        let rl = manifest.rate_limit.expect("rate_limit section");
        assert!(
            rl.tiers.is_empty(),
            "no tiers section should yield empty map"
        );
    }
}
