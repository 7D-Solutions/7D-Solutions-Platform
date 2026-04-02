//! Integration tests proving SDK auth works for verticals.
//!
//! Tests the full chain:
//! 1. JWKS endpoint -> JwtVerifier (real HTTP fetch, not mocked)
//! 2. optional_claims_mw extracts VerifiedClaims into request extensions
//! 3. RequirePermissionsLayer enforces permissions (401/403)
//! 4. Requests without JWT pass through to unprotected routes
//!
//! Bead: bd-47mzv

use std::sync::Arc;

use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use security::{optional_claims_mw, JwtVerifier, RequirePermissionsLayer, VerifiedClaims};
use serde::Serialize;
use uuid::Uuid;

// ── Helpers ──────────────────────────────────────────────────────────

fn make_keys() -> (EncodingKey, String, RsaPrivateKey) {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key gen");
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("PEM encode");
    let pub_pem = priv_key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .expect("public PEM");
    let enc = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
    (enc, pub_pem, priv_key)
}

fn rsa_to_jwks_json(priv_key: &RsaPrivateKey) -> String {
    let pub_key = priv_key.to_public_key();
    let n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
    format!(
        r#"{{"keys":[{{"kty":"RSA","use":"sig","alg":"RS256","n":"{n}","e":"{e}","kid":"test-1"}}]}}"#
    )
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn sign_token(enc: &EncodingKey, perms: Vec<String>) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        roles: vec!["operator".into()],
        perms,
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, enc).expect("sign token")
}

/// Build a router that mimics how a vertical would wire SDK auth.
fn build_test_router(verifier: Option<Arc<JwtVerifier>>) -> Router {
    // Protected mutation route — requires "vertical.mutate" permission
    let protected = Router::new()
        .route("/api/v1/orders", post(create_order))
        .route_layer(RequirePermissionsLayer::new(&["vertical.mutate"]));

    // Public read route — no permission required
    let public = Router::new().route("/api/v1/orders", get(list_orders));

    // Health route — always accessible
    let health = Router::new().route("/healthz", get(healthz));

    Router::new()
        .merge(protected)
        .merge(public)
        .merge(health)
        .layer(axum::middleware::from_fn_with_state(
            verifier,
            optional_claims_mw,
        ))
}

async fn create_order(Extension(claims): Extension<VerifiedClaims>) -> impl IntoResponse {
    (
        StatusCode::CREATED,
        format!("created by user={}", claims.user_id),
    )
}

async fn list_orders() -> impl IntoResponse {
    (StatusCode::OK, "orders listed")
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ── Tests ────────────────────────────────────────────────────────────

/// JWKS endpoint fetch: JwtVerifier successfully fetches keys from a real
/// HTTP server and verifies a token signed with the corresponding private key.
#[tokio::test]
async fn jwks_endpoint_fetch_and_verify() {
    let (enc, _pub_pem, priv_key) = make_keys();
    let jwks_json = rsa_to_jwks_json(&priv_key);

    // Start a real HTTP server serving JWKS
    let jwks_app = Router::new().route(
        "/.well-known/jwks.json",
        get(move || {
            let body = jwks_json.clone();
            async move { (StatusCode::OK, [("content-type", "application/json")], body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, jwks_app).await.ok();
    });

    let jwks_url = format!("http://{}/.well-known/jwks.json", addr);
    let verifier = JwtVerifier::from_jwks_url(
        &jwks_url,
        std::time::Duration::from_secs(300),
        false,
    )
    .await
    .expect("JwtVerifier from JWKS");

    let token = sign_token(&enc, vec!["ar.mutate".into()]);
    let verified = verifier.verify(&token).expect("verify token from JWKS");
    assert!(!verified.roles.is_empty());
    assert!(verified.perms.contains(&"ar.mutate".to_string()));
}

/// Full middleware chain: valid JWT with correct permissions -> 201 Created.
#[tokio::test]
async fn valid_jwt_with_permission_gets_201() {
    let (enc, pub_pem, _) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).expect("verifier"));
    let app = build_test_router(Some(verifier));

    let token = sign_token(&enc, vec!["vertical.mutate".into()]);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/orders")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

/// Valid JWT but missing required permission -> 403 Forbidden.
#[tokio::test]
async fn valid_jwt_missing_permission_gets_403() {
    let (enc, pub_pem, _) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).expect("verifier"));
    let app = build_test_router(Some(verifier));

    let token = sign_token(&enc, vec!["ar.read".into()]); // wrong permission
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/orders")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// No JWT on protected route -> 401 Unauthorized.
#[tokio::test]
async fn no_jwt_on_protected_route_gets_401() {
    let (_enc, pub_pem, _) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).expect("verifier"));
    let app = build_test_router(Some(verifier));

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/orders")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// No JWT on public route -> 200 OK (permissive middleware).
#[tokio::test]
async fn no_jwt_on_public_route_gets_200() {
    let (_enc, pub_pem, _) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).expect("verifier"));
    let app = build_test_router(Some(verifier));

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/orders")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// Health endpoint works with no JWT and no verifier.
#[tokio::test]
async fn health_works_without_verifier() {
    let app = build_test_router(None);

    let req = Request::builder()
        .method("GET")
        .uri("/healthz")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// JWKS fetch + full middleware chain: end-to-end from JWKS server to
/// protected route with permission enforcement.
#[tokio::test]
async fn jwks_endpoint_to_protected_route_e2e() {
    let (enc, _pub_pem, priv_key) = make_keys();
    let jwks_json = rsa_to_jwks_json(&priv_key);

    // Start JWKS server
    let jwks_app = Router::new().route(
        "/.well-known/jwks.json",
        get(move || {
            let body = jwks_json.clone();
            async move { (StatusCode::OK, [("content-type", "application/json")], body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, jwks_app).await.ok();
    });

    // Create verifier from JWKS endpoint (real HTTP fetch)
    let jwks_url = format!("http://{}/.well-known/jwks.json", addr);
    let verifier = Arc::new(
        JwtVerifier::from_jwks_url(&jwks_url, std::time::Duration::from_secs(300), false)
            .await
            .expect("verifier from JWKS"),
    );

    let app = build_test_router(Some(verifier));

    // Valid JWT with correct permission -> 201
    let token = sign_token(&enc, vec!["vertical.mutate".into()]);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/orders")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}
