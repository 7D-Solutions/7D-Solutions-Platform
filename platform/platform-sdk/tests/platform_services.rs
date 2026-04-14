//! Integration tests for the `[platform.services]` manifest section and
//! `ModuleContext::platform_client::<T>()` typed client construction.

use platform_sdk::{PlatformClient, PlatformService};

// ── Fake typed client for testing ────────────────────────────────────

struct FakePartiesClient {
    _client: PlatformClient,
}

impl PlatformService for FakePartiesClient {
    const SERVICE_NAME: &'static str = "party";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self { _client: client }
    }
}

struct FakeInventoryClient {
    _client: PlatformClient,
}

impl PlatformService for FakeInventoryClient {
    const SERVICE_NAME: &'static str = "inventory";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self { _client: client }
    }
}

// ── Manifest parsing ─────────────────────────────────────────────────

#[test]
fn manifest_with_platform_services_parses() {
    let toml_str = r#"
[module]
name = "test-vertical"

[platform.services]
party     = { enabled = true, default_url = "http://localhost:8098" }
inventory = { enabled = true, timeout_secs = 60, default_url = "http://localhost:8092" }
bom       = { enabled = false }
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None).expect("manifest should parse");
    let platform = manifest.platform.expect("platform section");
    assert_eq!(platform.services.len(), 3);
    assert!(platform.services["party"].enabled);
    assert_eq!(platform.services["inventory"].timeout_secs, Some(60));
    assert!(!platform.services["bom"].enabled);
}

#[test]
fn manifest_without_platform_section_parses() {
    let toml_str = r#"
[module]
name = "legacy-module"
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None).expect("manifest should parse");
    assert!(manifest.platform.is_none());
}

// ── PlatformServices construction ────────────────────────────────────

#[test]
fn platform_services_builds_from_manifest() {
    let toml_str = r#"
[module]
name = "test"

[platform.services]
party     = { enabled = true, default_url = "http://party:8098" }
inventory = { enabled = true, timeout_secs = 45, default_url = "http://inventory:8092" }
bom       = { enabled = false }
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None).unwrap();
    let services = platform_sdk::platform_services::PlatformServices::from_manifest(
        manifest.platform.as_ref(),
        "test",
    )
    .unwrap();

    // 2 enabled services (bom is disabled)
    assert_eq!(services.len(), 2);
    assert!(services.get("party").is_some());
    assert!(services.get("inventory").is_some());
    assert!(services.get("bom").is_none());
}

// ── ModuleContext::platform_client ────────────────────────────────────

#[test]
fn context_platform_client_constructs_typed_client() {
    let toml_str = r#"
[module]
name = "ctx-test"

[platform.services]
party = { enabled = true, default_url = "http://localhost:8098" }
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None).unwrap();
    let services = platform_sdk::platform_services::PlatformServices::from_manifest(
        manifest.platform.as_ref(),
        "ctx-test",
    )
    .unwrap();

    let client = services.get("party").expect("party service");
    let _typed: FakePartiesClient = FakePartiesClient::from_platform_client(client.clone());
}

#[test]
fn platform_services_missing_env_var_with_no_default_fails() {
    std::env::remove_var("NONEXISTENT_BASE_URL");

    let toml_str = r#"
[module]
name = "fail-test"

[platform.services]
nonexistent = { enabled = true }
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None).unwrap();
    let err = platform_sdk::platform_services::PlatformServices::from_manifest(
        manifest.platform.as_ref(),
        "fail-test",
    )
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("NONEXISTENT_BASE_URL"), "got: {msg}");
}

#[test]
fn platform_services_default_url_used_when_env_missing() {
    std::env::remove_var("GL_BASE_URL");

    let toml_str = r#"
[module]
name = "default-url-test"

[platform.services]
gl = { enabled = true, default_url = "http://localhost:8090" }
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None).unwrap();
    let services = platform_sdk::platform_services::PlatformServices::from_manifest(
        manifest.platform.as_ref(),
        "default-url-test",
    )
    .unwrap();

    assert!(services.get("gl").is_some());
}

#[test]
fn platform_service_trait_wires_correctly() {
    let client = PlatformClient::new("http://localhost:8098".to_string());
    let typed = FakePartiesClient::from_platform_client(client);
    // Just verify construction doesn't panic
    assert_eq!(FakePartiesClient::SERVICE_NAME, "party");
    drop(typed);
}
