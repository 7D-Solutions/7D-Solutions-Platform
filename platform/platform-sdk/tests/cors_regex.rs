//! Integration tests for CORS layer built from manifest settings.
//!
//! Tests the three CORS paths:
//! 1. cors.origin_pattern (regex predicate)
//! 2. cors.origins (explicit list)
//! 3. env-var fallback (no cors section in manifest)

use axum::routing::get;
use axum::Router;
use platform_sdk::Manifest;
use std::fs;
use tempfile::TempDir;
use tower_http::cors::{AllowOrigin, CorsLayer};

fn write_manifest(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let path = dir.path().join("module.toml");
    fs::write(&path, content).expect("write test manifest");
    path
}

/// Build a CORS layer from a manifest, mirroring the logic in startup.rs.
/// This duplicates the priority chain so we can test each path in isolation.
fn build_cors_layer_from_manifest(manifest: &Manifest) -> CorsLayer {
    let env_val = std::env::var("ENV").unwrap_or_else(|_| "development".to_string());

    // 1. Manifest cors.origin_pattern → regex predicate
    if let Some(ref pattern) = manifest.cors.as_ref().and_then(|c| c.origin_pattern.clone()) {
        let re = regex::Regex::new(pattern).expect("manifest validate() ensures valid regex");
        return CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _| {
                origin.to_str().map_or(false, |s| re.is_match(s))
            }))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
            .allow_credentials(false);
    }

    // 2. Manifest cors.origins → explicit list
    if let Some(ref origins) = manifest.cors.as_ref().and_then(|c| c.origins.clone()) {
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        return CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
            .allow_credentials(false);
    }

    // 3. Fallback: CORS_ORIGINS env var
    let cors_env = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| "*".to_string());
    let origins: Vec<String> = cors_env
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let is_wildcard = origins.len() == 1 && origins[0] == "*";

    if is_wildcard && env_val != "development" {
        // warn omitted in tests
    }

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new().allow_origin(parsed)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
        .allow_credentials(false)
}

/// Helper: send an OPTIONS preflight request and return whether
/// the Access-Control-Allow-Origin header is present in the response.
async fn cors_allows(cors: CorsLayer, origin: &str) -> bool {
    let app = Router::new()
        .route("/test", get(|| async { "ok" }))
        .layer(cors);

    let request = axum::http::Request::builder()
        .method("OPTIONS")
        .uri("/test")
        .header("Origin", origin)
        .header("Access-Control-Request-Method", "GET")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
    response
        .headers()
        .get("access-control-allow-origin")
        .is_some()
}

// --- origin_pattern tests ---

#[tokio::test]
async fn origin_pattern_matches_subdomain() {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-regex-test"

[cors]
origin_pattern = "^https://.*\\.example\\.com$"
"#,
    );

    let manifest = Manifest::from_file(&path).unwrap();
    let cors = build_cors_layer_from_manifest(&manifest);

    assert!(cors_allows(cors, "https://app.example.com").await);
}

#[tokio::test]
async fn origin_pattern_rejects_non_matching() {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-regex-test"

[cors]
origin_pattern = "^https://.*\\.example\\.com$"
"#,
    );

    let manifest = Manifest::from_file(&path).unwrap();
    let cors = build_cors_layer_from_manifest(&manifest);

    assert!(!cors_allows(cors, "https://evil.com").await);
}

#[tokio::test]
async fn origin_pattern_matches_deep_subdomain() {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-regex-test"

[cors]
origin_pattern = "^https://.*\\.example\\.com$"
"#,
    );

    let manifest = Manifest::from_file(&path).unwrap();
    let cors = build_cors_layer_from_manifest(&manifest);

    assert!(cors_allows(cors, "https://staging.app.example.com").await);
}

// --- cors.origins tests ---

#[tokio::test]
async fn origins_list_allows_listed_origin() {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-list-test"

[cors]
origins = ["https://app.example.com", "https://admin.example.com"]
"#,
    );

    let manifest = Manifest::from_file(&path).unwrap();
    let cors = build_cors_layer_from_manifest(&manifest);

    assert!(cors_allows(cors, "https://app.example.com").await);
}

#[tokio::test]
async fn origins_list_rejects_unlisted_origin() {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-list-test"

[cors]
origins = ["https://app.example.com"]
"#,
    );

    let manifest = Manifest::from_file(&path).unwrap();
    let cors = build_cors_layer_from_manifest(&manifest);

    assert!(!cors_allows(cors, "https://evil.com").await);
}

// --- env-var fallback tests ---

#[tokio::test]
async fn no_cors_section_falls_back_to_env_wildcard() {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-fallback"
"#,
    );

    let manifest = Manifest::from_file(&path).unwrap();
    assert!(manifest.cors.is_none());

    // Default CORS_ORIGINS is "*" → allows any origin
    let cors = build_cors_layer_from_manifest(&manifest);
    assert!(cors_allows(cors, "https://anything.com").await);
}
