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
