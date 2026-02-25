//! Integration tests for RBAC permission enforcement.
//!
//! Tests the full middleware stack: JWT verification → claims extraction →
//! permission enforcement via RequirePermissionsLayer. Uses real RSA
//! keypairs — no mocks or stubs.

use axum::{body::Body, routing::get, Router};
use chrono::{Duration, Utc};
use http::Request;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serde::Serialize;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use security::authz_middleware::{ClaimsLayer, RequirePermissionsLayer};
use security::claims::JwtVerifier;
use security::rbac::{Operation, RbacPolicy, Role};

// ── Helpers ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

struct TestKeys {
    encoding: EncodingKey,
    verifier: Arc<JwtVerifier>,
}

fn make_keys() -> TestKeys {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
    TestKeys {
        encoding: EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap(),
        verifier: Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap()),
    }
}

fn sign(enc: &EncodingKey, claims: &TestClaims) -> String {
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), claims, enc).unwrap()
}

fn claims_with_perms(perms: Vec<String>) -> TestClaims {
    let now = Utc::now();
    TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        app_id: None,
        roles: vec!["admin".into()],
        perms,
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    }
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

// ── Middleware stack: ClaimsLayer + RequirePermissionsLayer ───────────────

fn guarded_app(keys: &TestKeys, required: &[&str]) -> Router {
    Router::new()
        .route("/protected", get(|| async { "ok" }))
        .route_layer(RequirePermissionsLayer::new(required))
        .layer(ClaimsLayer::permissive(keys.verifier.clone()))
}

#[tokio::test]
async fn grants_access_with_all_required_permissions() {
    let keys = make_keys();
    let claims = claims_with_perms(vec!["ar.mutate".into(), "gl.post".into()]);
    let token = sign(&keys.encoding, &claims);
    let app = guarded_app(&keys, &["ar.mutate", "gl.post"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("authorization", bearer(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn denies_with_missing_permission() {
    let keys = make_keys();
    let claims = claims_with_perms(vec!["ar.mutate".into()]); // missing gl.post
    let token = sign(&keys.encoding, &claims);
    let app = guarded_app(&keys, &["ar.mutate", "gl.post"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("authorization", bearer(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn denies_with_no_token() {
    let keys = make_keys();
    let app = guarded_app(&keys, &["ar.mutate"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn denies_with_invalid_token() {
    let keys = make_keys();
    let app = guarded_app(&keys, &["ar.mutate"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("authorization", "Bearer garbage-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Permissive ClaimsLayer lets it through without claims, but
    // RequirePermissionsLayer returns 401 because no claims present
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn denies_with_expired_token() {
    let keys = make_keys();
    let now = Utc::now();
    let mut claims = claims_with_perms(vec!["ar.mutate".into()]);
    claims.iat = (now - Duration::hours(1)).timestamp();
    claims.exp = (now - Duration::minutes(5)).timestamp();
    let token = sign(&keys.encoding, &claims);
    let app = guarded_app(&keys, &["ar.mutate"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("authorization", bearer(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Expired token → no claims → 401
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn superset_permissions_grants_access() {
    let keys = make_keys();
    let claims = claims_with_perms(vec![
        "ar.mutate".into(),
        "gl.post".into(),
        "inventory.read".into(),
    ]);
    let token = sign(&keys.encoding, &claims);
    let app = guarded_app(&keys, &["ar.mutate"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("authorization", bearer(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn empty_perms_denied_when_route_requires_perms() {
    let keys = make_keys();
    let claims = claims_with_perms(vec![]);
    let token = sign(&keys.encoding, &claims);
    let app = guarded_app(&keys, &["ar.mutate"]);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/protected")
                .header("authorization", bearer(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
}

// ── Strict mode ClaimsLayer ──────────────────────────────────────────────

#[tokio::test]
async fn strict_mode_rejects_missing_token() {
    let keys = make_keys();
    let app = Router::new()
        .route("/strict", get(|| async { "ok" }))
        .layer(ClaimsLayer::strict(keys.verifier));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/strict")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn strict_mode_accepts_valid_token() {
    let keys = make_keys();
    let claims = claims_with_perms(vec![]);
    let token = sign(&keys.encoding, &claims);
    let app = Router::new()
        .route("/strict", get(|| async { "ok" }))
        .layer(ClaimsLayer::strict(keys.verifier));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/strict")
                .header("authorization", bearer(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

// ── RbacPolicy direct tests (cross-role matrix) ─────────────────────────

#[test]
fn admin_can_do_everything() {
    let ops = [
        Operation::TenantSuspend,
        Operation::TenantDeprovision,
        Operation::ProjectionRebuild,
        Operation::ProjectionVerify,
        Operation::ProjectionStatus,
        Operation::ProjectionList,
        Operation::FleetMigrate,
    ];
    for op in &ops {
        assert!(
            RbacPolicy::has_permission(Role::Admin, *op),
            "Admin should have permission for {:?}",
            op
        );
    }
}

#[test]
fn operator_denied_deprovision_and_fleet_migrate() {
    assert!(!RbacPolicy::has_permission(Role::Operator, Operation::TenantDeprovision));
    assert!(!RbacPolicy::has_permission(Role::Operator, Operation::FleetMigrate));
}

#[test]
fn operator_allowed_suspend_and_projections() {
    assert!(RbacPolicy::has_permission(Role::Operator, Operation::TenantSuspend));
    assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionRebuild));
    assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionVerify));
    assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionStatus));
    assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionList));
}

#[test]
fn auditor_read_only() {
    assert!(RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionVerify));
    assert!(RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionStatus));
    assert!(RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionList));

    assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::TenantSuspend));
    assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::TenantDeprovision));
    assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionRebuild));
    assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::FleetMigrate));
}

#[test]
fn authorize_returns_error_with_context() {
    let err = RbacPolicy::authorize(
        Role::Auditor,
        Operation::TenantSuspend,
        "auditor-1",
        "tenant-abc",
    )
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("Auditor"), "error should mention role");
    assert!(msg.contains("TenantSuspend"), "error should mention operation");
    assert!(msg.contains("auditor-1"), "error should mention actor");
    assert!(msg.contains("tenant-abc"), "error should mention resource");
}

#[test]
fn authorize_success_returns_ok() {
    assert!(RbacPolicy::authorize(
        Role::Admin,
        Operation::FleetMigrate,
        "admin-1",
        "fleet",
    )
    .is_ok());
}

#[test]
fn role_parsing_case_insensitive() {
    assert_eq!(Role::from_str("Admin"), Some(Role::Admin));
    assert_eq!(Role::from_str("OPERATOR"), Some(Role::Operator));
    assert_eq!(Role::from_str("auditor"), Some(Role::Auditor));
    assert_eq!(Role::from_str("bogus"), None);
}
