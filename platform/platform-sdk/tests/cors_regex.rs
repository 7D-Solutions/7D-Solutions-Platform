//! Integration tests for CORS layer built from manifest settings.
//!
//! Tests the manifest-driven CORS policy (bd-7btgv):
//! - Manifest `[cors]` section is authoritative.
//! - `CORS_ORIGINS` env var is an operator override only when the manifest has no `[cors]`.
//! - Wildcard anywhere — in manifest or in operator override — is an error.
//! - Missing declaration in both places is an error.

use axum::routing::get;
use axum::Router;
use platform_sdk::{build_cors_layer, Manifest, StartupError};
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;
use tower_http::cors::CorsLayer;

/// Serializes env-var mutations across fail-closed tests so they don't race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that sets `CORS_ORIGINS` on entry and restores its prior value on drop.
struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prior_origins: Option<String>,
}

impl EnvGuard {
    fn new(origins: Option<&str>) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_origins = std::env::var("CORS_ORIGINS").ok();
        match origins {
            Some(v) => std::env::set_var("CORS_ORIGINS", v),
            None => std::env::remove_var("CORS_ORIGINS"),
        }
        Self {
            _lock: lock,
            prior_origins,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prior_origins {
            Some(v) => std::env::set_var("CORS_ORIGINS", v),
            None => std::env::remove_var("CORS_ORIGINS"),
        }
    }
}

fn write_manifest(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let path = dir.path().join("module.toml");
    fs::write(&path, content).expect("write test manifest");
    path
}

fn manifest_with(cors_section: &str) -> (TempDir, Manifest) {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        &format!(
            r#"
[module]
name = "cors-prod-guard"

{cors_section}
"#,
        ),
    );
    let manifest = Manifest::from_file(&path).unwrap();
    (dir, manifest)
}

fn bare_manifest() -> (TempDir, Manifest) {
    let dir = TempDir::new().unwrap();
    let path = write_manifest(
        &dir,
        r#"
[module]
name = "cors-prod-guard"
"#,
    );
    let manifest = Manifest::from_file(&path).unwrap();
    (dir, manifest)
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

// --- manifest-driven CORS policy (bd-7btgv) --------------------------------

#[tokio::test]
async fn test_no_manifest_no_env_errors() {
    let _g = EnvGuard::new(None);
    let (_dir, manifest) = bare_manifest();
    let err = build_cors_layer(&manifest).expect_err("no manifest + no env must fail");
    assert!(matches!(err, StartupError::Config(_)), "expected Config, got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("cors-prod-guard"),
        "error must name the module: {msg}"
    );
}

#[tokio::test]
async fn test_manifest_wildcard_errors() {
    let _g = EnvGuard::new(Some("https://anything.example.com"));
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origins = ["*"]"#,
    );
    let err =
        build_cors_layer(&manifest).expect_err("manifest wildcard origins must fail");
    assert!(matches!(err, StartupError::Config(_)), "expected Config, got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("manifest.cors.origins"),
        "error must name manifest.cors.origins: {msg}"
    );
}

#[tokio::test]
async fn test_manifest_wildcard_pattern_errors() {
    let _g = EnvGuard::new(Some("https://anything.example.com"));
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origin_pattern = ".*""#,
    );
    let err =
        build_cors_layer(&manifest).expect_err("manifest wildcard pattern must fail");
    assert!(matches!(err, StartupError::Config(_)), "expected Config, got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("manifest.cors.origin_pattern"),
        "error must name manifest.cors.origin_pattern: {msg}"
    );
}

#[tokio::test]
async fn test_manifest_explicit_origins_ok() {
    // CORS_ORIGINS set to wildcard to prove it isn't consulted when manifest has origins.
    let _g = EnvGuard::new(Some("*"));
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origins = ["https://app.example.com"]"#,
    );
    let cors = build_cors_layer(&manifest).expect("explicit manifest origins must succeed");
    assert!(cors_allows(cors, "https://app.example.com").await);
}

#[tokio::test]
async fn test_manifest_empty_origins_ok() {
    let _g = EnvGuard::new(None);
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origins = []"#,
    );
    let cors = build_cors_layer(&manifest).expect("empty manifest origins must succeed");
    // Empty allow list → no cross-origin request is permitted.
    assert!(!cors_allows(cors, "https://app.example.com").await);
}

#[tokio::test]
async fn test_env_override_explicit_ok() {
    let _g = EnvGuard::new(Some("https://a.example.com,https://b.example.com"));
    let (_dir, manifest) = bare_manifest();
    let cors = build_cors_layer(&manifest).expect("explicit env override must succeed");
    assert!(cors_allows(cors, "https://a.example.com").await);
}

#[tokio::test]
async fn test_env_override_wildcard_errors() {
    let _g = EnvGuard::new(Some("*"));
    let (_dir, manifest) = bare_manifest();
    let err =
        build_cors_layer(&manifest).expect_err("wildcard env override must fail");
    assert!(matches!(err, StartupError::Config(_)), "expected Config, got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("CORS_ORIGINS"),
        "error must name CORS_ORIGINS: {msg}"
    );
}

// --- regression guards for origin_pattern and origins behavior --------------

#[tokio::test]
async fn origin_pattern_matches_subdomain() {
    let _g = EnvGuard::new(None);
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origin_pattern = "^https://.*\\.example\\.com$""#,
    );
    let cors = build_cors_layer(&manifest).expect("valid regex pattern");
    assert!(cors_allows(cors, "https://app.example.com").await);
}

#[tokio::test]
async fn origin_pattern_rejects_non_matching() {
    let _g = EnvGuard::new(None);
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origin_pattern = "^https://.*\\.example\\.com$""#,
    );
    let cors = build_cors_layer(&manifest).expect("valid regex pattern");
    assert!(!cors_allows(cors, "https://evil.com").await);
}

#[tokio::test]
async fn origins_list_rejects_unlisted_origin() {
    let _g = EnvGuard::new(None);
    let (_dir, manifest) = manifest_with(
        r#"[cors]
origins = ["https://app.example.com"]"#,
    );
    let cors = build_cors_layer(&manifest).expect("explicit origins");
    assert!(!cors_allows(cors, "https://evil.com").await);
}
