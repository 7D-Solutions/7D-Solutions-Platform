//! Integration tests for module.toml manifest parsing and validation.

use platform_sdk::Manifest;
use std::fs;
use tempfile::TempDir;

fn write_manifest(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let path = dir.path().join("module.toml");
    fs::write(&path, content).expect("write test manifest");
    path
}

#[test]
fn valid_manifest_parses_all_fields() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("db/migrations")).expect("create migrations dir");

    let path = write_manifest(
        &dir,
        r#"
[module]
name = "party"
version = "2.3.3"
description = "Party master data"

[server]
host = "0.0.0.0"
port = 8098

[database]
migrations = "./db/migrations"
auto_migrate = true

[bus]
type = "inmemory"

[sdk]
min_version = "0.1.0"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("parse valid manifest");
    assert_eq!(manifest.module.name, "party");
    assert_eq!(manifest.module.version.as_deref(), Some("2.3.3"));
    assert_eq!(
        manifest.module.description.as_deref(),
        Some("Party master data")
    );
    assert_eq!(manifest.server.host, "0.0.0.0");
    assert_eq!(manifest.server.port, 8098);

    let db = manifest.database.expect("database section");
    assert_eq!(db.migrations, "./db/migrations");
    assert!(db.auto_migrate);

    let bus = manifest.bus.expect("bus section");
    assert_eq!(bus.bus_type, "inmemory");

    let sdk = manifest.sdk.expect("sdk section");
    assert_eq!(sdk.min_version.as_deref(), Some("0.1.0"));
}

#[test]
fn minimal_manifest_uses_defaults() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "minimal"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("parse minimal manifest");
    assert_eq!(manifest.module.name, "minimal");
    assert_eq!(manifest.server.host, "0.0.0.0");
    assert_eq!(manifest.server.port, 8080);
    assert!(manifest.database.is_none());
    assert!(manifest.bus.is_none());
    assert!(manifest.sdk.is_none());
}

#[test]
fn empty_module_name_returns_typed_error() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = ""
"#,
    );

    let err = Manifest::from_file(&path).expect_err("empty name should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("must not be empty"),
        "expected name validation error, got: {}",
        msg
    );
}

#[test]
fn invalid_bus_type_returns_typed_error() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "test"

[bus]
type = "kafka"
"#,
    );

    let err = Manifest::from_file(&path).expect_err("kafka should fail");
    let msg = err.to_string();
    assert!(msg.contains("kafka"), "expected bus type error, got: {}", msg);
}

#[test]
fn invalid_toml_returns_parse_error() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(&dir, "not valid toml [[[");

    let err = Manifest::from_file(&path).expect_err("invalid TOML should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("parse"),
        "expected parse error, got: {}",
        msg
    );
}

#[test]
fn missing_file_returns_io_error() {
    let path = std::path::PathBuf::from("/tmp/nonexistent-module.toml");
    let err = Manifest::from_file(&path).expect_err("missing file should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("failed to read"),
        "expected IO error, got: {}",
        msg
    );
}

#[test]
fn missing_migrations_path_returns_typed_error() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "test"

[database]
migrations = "./nonexistent/migrations"
auto_migrate = true
"#,
    );

    let err = Manifest::from_file(&path).expect_err("missing migrations should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("does not exist"),
        "expected migration path error, got: {}",
        msg
    );
}

#[test]
fn sdk_version_compat_passes_for_current() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "compat-ok"

[sdk]
min_version = "0.1.0"
"#,
    );

    Manifest::from_file(&path).expect("current version should pass");
}

#[test]
fn sdk_version_compat_fails_for_future() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "compat-fail"

[sdk]
min_version = "99.0.0"
"#,
    );

    let err = Manifest::from_file(&path).expect_err("future version should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("99.0.0"),
        "expected version compat error, got: {}",
        msg
    );
}

#[test]
fn invalid_semver_returns_typed_error() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "bad-semver"

[sdk]
min_version = "not.a.version"
"#,
    );

    let err = Manifest::from_file(&path).expect_err("bad semver should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("not valid semver"),
        "expected semver error, got: {}",
        msg
    );
}

#[test]
fn unknown_keys_warn_but_parse_successfully() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "extras"
custom_field = "hello"

[unknown_section]
key = "value"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("unknown keys should parse");
    assert_eq!(manifest.module.name, "extras");
    assert!(manifest.extra.contains_key("unknown_section"));
    assert!(manifest.module.extra.contains_key("custom_field"));
}

#[test]
fn nats_bus_type_parses() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "nats-module"

[bus]
type = "nats"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("nats manifest should parse");
    assert_eq!(manifest.bus.expect("bus section").bus_type, "nats");
}

#[test]
fn bus_type_is_case_insensitive() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "case-test"

[bus]
type = "NATS"
"#,
    );

    Manifest::from_file(&path).expect("uppercase NATS should parse");
}

// --- Auth section ---

#[test]
fn auth_section_parses_with_defaults() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "auth-defaults"

[auth]
"#,
    );

    let manifest = Manifest::from_file(&path).expect("auth defaults should parse");
    let auth = manifest.auth.expect("auth section");
    assert!(auth.jwks_url.is_none());
    assert_eq!(auth.refresh_interval, "5m");
    assert!(auth.fallback_to_env);
    assert!(auth.enabled);
}

#[test]
fn auth_section_parses_all_fields() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "auth-full"

[auth]
jwks_url = "https://example.com/.well-known/jwks.json"
refresh_interval = "10m"
fallback_to_env = false
enabled = true
"#,
    );

    let manifest = Manifest::from_file(&path).expect("full auth should parse");
    let auth = manifest.auth.expect("auth section");
    assert_eq!(
        auth.jwks_url.as_deref(),
        Some("https://example.com/.well-known/jwks.json")
    );
    assert_eq!(auth.refresh_interval, "10m");
    assert!(!auth.fallback_to_env);
    assert!(auth.enabled);
}

#[test]
fn auth_jwks_url_with_enabled_false_fails() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "auth-conflict"

[auth]
jwks_url = "https://example.com/jwks"
enabled = false
"#,
    );

    let err = Manifest::from_file(&path).expect_err("jwks_url + disabled should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("jwks_url") && msg.contains("enabled"),
        "expected auth conflict error, got: {}",
        msg
    );
}

#[test]
fn auth_required_defaults_to_true() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "auth-required-default"

[auth]
"#,
    );

    let manifest = Manifest::from_file(&path).expect("auth section should parse");
    let auth = manifest.auth.expect("auth section");
    assert!(auth.required, "auth.required should default to true");
}

#[test]
fn auth_required_false_parses() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "auth-optional"

[auth]
required = false
enabled = false
"#,
    );

    let manifest = Manifest::from_file(&path).expect("auth required=false should parse");
    let auth = manifest.auth.expect("auth section");
    assert!(!auth.required);
}

#[test]
fn auth_required_true_with_enabled_false_fails() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "auth-contradiction"

[auth]
required = true
enabled = false
"#,
    );

    let err = Manifest::from_file(&path).expect_err("required+disabled should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("required") && msg.contains("enabled"),
        "expected auth contradiction error, got: {}",
        msg
    );
}

#[test]
fn missing_auth_section_uses_none() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "no-auth"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("no auth should parse");
    assert!(manifest.auth.is_none());
}

// --- CORS section ---

#[test]
fn cors_section_parses_origins() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-origins"

[cors]
origins = ["https://example.com", "https://app.example.com"]
"#,
    );

    let manifest = Manifest::from_file(&path).expect("cors origins should parse");
    let cors = manifest.cors.expect("cors section");
    let origins = cors.origins.expect("origins list");
    assert_eq!(origins.len(), 2);
    assert!(cors.origin_pattern.is_none());
}

#[test]
fn cors_section_parses_origin_pattern() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-pattern"

[cors]
origin_pattern = "^https://.*\\.example\\.com$"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("cors pattern should parse");
    let cors = manifest.cors.expect("cors section");
    assert!(cors.origins.is_none());
    assert!(cors.origin_pattern.is_some());
}

#[test]
fn cors_origins_and_pattern_both_set_fails() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-conflict"

[cors]
origins = ["https://example.com"]
origin_pattern = "^https://.*$"
"#,
    );

    let err = Manifest::from_file(&path).expect_err("both cors modes should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("mutually exclusive"),
        "expected cors conflict error, got: {}",
        msg
    );
}

#[test]
fn cors_invalid_regex_fails() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-bad-regex"

[cors]
origin_pattern = "[invalid"
"#,
    );

    let err = Manifest::from_file(&path).expect_err("bad regex should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("not a valid regex"),
        "expected regex error, got: {}",
        msg
    );
}

// --- Health section ---

#[test]
fn health_section_parses_known_deps() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "health-ok"

[health]
dependencies = ["postgres", "nats"]
"#,
    );

    let manifest = Manifest::from_file(&path).expect("health deps should parse");
    let health = manifest.health.expect("health section");
    assert_eq!(health.dependencies, vec!["postgres", "nats"]);
}

#[test]
fn health_unknown_dependency_fails() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "health-bad"

[health]
dependencies = ["postgres", "redis"]
"#,
    );

    let err = Manifest::from_file(&path).expect_err("unknown dep should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("redis"),
        "expected unknown dep error, got: {}",
        msg
    );
}

#[test]
fn health_empty_deps_parses() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "health-empty"

[health]
dependencies = []
"#,
    );

    let manifest = Manifest::from_file(&path).expect("empty deps should parse");
    let health = manifest.health.expect("health section");
    assert!(health.dependencies.is_empty());
}

// --- Rate limit section ---

#[test]
fn rate_limit_section_parses_with_defaults() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "rate-defaults"

[rate_limit]
"#,
    );

    let manifest = Manifest::from_file(&path).expect("rate limit defaults should parse");
    let rl = manifest.rate_limit.expect("rate_limit section");
    assert_eq!(rl.requests_per_second, 100);
    assert_eq!(rl.burst, 200);
}

#[test]
fn rate_limit_section_parses_custom_values() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "rate-custom"

[rate_limit]
requests_per_second = 50
burst = 100
"#,
    );

    let manifest = Manifest::from_file(&path).expect("custom rate limit should parse");
    let rl = manifest.rate_limit.expect("rate_limit section");
    assert_eq!(rl.requests_per_second, 50);
    assert_eq!(rl.burst, 100);
}

// --- Server extensions ---

#[test]
fn server_section_has_new_defaults() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "server-defaults"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("server defaults should parse");
    assert_eq!(manifest.server.body_limit, "2mb");
    assert_eq!(manifest.server.request_timeout, "30s");
}

#[test]
fn server_section_parses_custom_limits() {
    let dir = TempDir::new().expect("tempdir");
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "server-custom"

[server]
host = "0.0.0.0"
port = 9000
body_limit = "10mb"
request_timeout = "60s"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("custom server should parse");
    assert_eq!(manifest.server.body_limit, "10mb");
    assert_eq!(manifest.server.request_timeout, "60s");
}

// --- Database extensions ---

#[test]
fn database_section_has_pool_defaults() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("db/migrations")).expect("create migrations dir");

    let path = write_manifest(
        &dir,
        r#"
[module]
name = "db-defaults"

[database]
migrations = "./db/migrations"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("db defaults should parse");
    let db = manifest.database.expect("database section");
    assert_eq!(db.pool_min, 5);
    assert_eq!(db.pool_max, 20);
}

#[test]
fn database_section_parses_custom_pool() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("db/migrations")).expect("create migrations dir");

    let path = write_manifest(
        &dir,
        r#"
[module]
name = "db-custom"

[database]
migrations = "./db/migrations"
pool_min = 2
pool_max = 50
"#,
    );

    let manifest = Manifest::from_file(&path).expect("custom pool should parse");
    let db = manifest.database.expect("database section");
    assert_eq!(db.pool_min, 2);
    assert_eq!(db.pool_max, 50);
}

// --- Full manifest with all new sections ---

#[test]
fn full_manifest_with_all_sections_parses() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("db/migrations")).expect("create migrations dir");

    let path = write_manifest(
        &dir,
        r#"
[module]
name = "full"
version = "1.0.0"
description = "Full manifest test"

[server]
host = "0.0.0.0"
port = 8080
body_limit = "5mb"
request_timeout = "45s"

[database]
migrations = "./db/migrations"
auto_migrate = true
pool_min = 10
pool_max = 30

[bus]
type = "nats"

[events.publish]
outbox_table = "outbox"

[sdk]
min_version = "0.1.0"

[auth]
jwks_url = "https://example.com/jwks"
refresh_interval = "10m"
fallback_to_env = false
enabled = true

[cors]
origins = ["https://example.com"]

[health]
dependencies = ["postgres", "nats"]

[rate_limit]
requests_per_second = 200
burst = 400
"#,
    );

    let manifest = Manifest::from_file(&path).expect("full manifest should parse");
    assert_eq!(manifest.module.name, "full");
    assert!(manifest.auth.is_some());
    assert!(manifest.cors.is_some());
    assert!(manifest.health.is_some());
    assert!(manifest.rate_limit.is_some());
}

// --- Backward compatibility ---

#[test]
fn existing_module_toml_without_new_sections_parses() {
    // Simulates an existing module.toml that predates the new sections.
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("db/migrations")).expect("create migrations dir");

    let path = write_manifest(
        &dir,
        r#"
[module]
name = "legacy"
version = "2.4.2"
description = "Legacy module"

[server]
host = "0.0.0.0"
port = 8098

[database]
migrations = "./db/migrations"
auto_migrate = true

[sdk]
min_version = "0.1.0"
"#,
    );

    let manifest = Manifest::from_file(&path).expect("legacy manifest should parse");
    assert_eq!(manifest.module.name, "legacy");
    assert!(manifest.auth.is_none());
    assert!(manifest.cors.is_none());
    assert!(manifest.health.is_none());
    assert!(manifest.rate_limit.is_none());
    // Server defaults should still work
    assert_eq!(manifest.server.body_limit, "2mb");
    assert_eq!(manifest.server.request_timeout, "30s");
}
