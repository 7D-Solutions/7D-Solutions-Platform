//! Integration tests for auth-kit.
//!
//! Tests that a vertical app using the auth kit can:
//! 1. Login via proxy and get a JWT
//! 2. Subsequent requests with that JWT produce VerifiedClaims in handlers
//! 3. Requests without JWT get 401

use std::sync::Arc;

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use uuid::Uuid;

use auth_kit::{AuthKit, ClaimsLayer, JwtVerifier, RequirePermissionsLayer, VerifiedClaims};

// ── Test helpers ──────────────────────────────────────────────────

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

fn make_keys() -> (EncodingKey, String) {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key gen");
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("PEM encode");
    let pub_pem = pub_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public PEM");
    let enc = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
    (enc, pub_pem)
}

fn sign_token(enc: &EncodingKey, claims: &TestClaims) -> String {
    let header = Header::new(Algorithm::RS256);
    jsonwebtoken::encode(&header, claims, enc).expect("sign token")
}

fn default_claims(perms: Vec<String>) -> TestClaims {
    let now = Utc::now();
    TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        roles: vec!["admin".into()],
        perms,
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    }
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

// ── Handler that reads VerifiedClaims ─────────────────────────────

#[derive(Serialize, Deserialize)]
struct WhoAmI {
    user_id: String,
    tenant_id: String,
}

async fn whoami(Extension(claims): Extension<VerifiedClaims>) -> impl IntoResponse {
    axum::Json(WhoAmI {
        user_id: claims.user_id.to_string(),
        tenant_id: claims.tenant_id.to_string(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────

/// Test that requests with a valid JWT produce VerifiedClaims in handlers.
#[tokio::test]
async fn valid_jwt_produces_verified_claims() {
    let (enc, pub_pem) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let app = Router::new()
        .route("/api/whoami", get(whoami))
        .route_layer(RequirePermissionsLayer::new(&["orders.read"]))
        .layer(ClaimsLayer::permissive(verifier));

    let claims = default_claims(vec!["orders.read".into()]);
    let token = sign_token(&enc, &claims);

    let req = axum::http::Request::builder()
        .uri("/api/whoami")
        .header("authorization", bearer(&token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let who: WhoAmI = serde_json::from_slice(&body).unwrap();
    assert_eq!(who.user_id, claims.sub);
    assert_eq!(who.tenant_id, claims.tenant_id);
}

/// Test that requests without JWT get 401.
#[tokio::test]
async fn missing_jwt_gets_401() {
    let (_, pub_pem) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let app = Router::new()
        .route("/api/whoami", get(whoami))
        .route_layer(RequirePermissionsLayer::new(&["orders.read"]))
        .layer(ClaimsLayer::permissive(verifier));

    let req = axum::http::Request::builder()
        .uri("/api/whoami")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Test that insufficient permissions get 403.
#[tokio::test]
async fn insufficient_permissions_gets_403() {
    let (enc, pub_pem) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let app = Router::new()
        .route("/api/whoami", get(whoami))
        .route_layer(RequirePermissionsLayer::new(&["admin.nuke"]))
        .layer(ClaimsLayer::permissive(verifier));

    let claims = default_claims(vec!["orders.read".into()]);
    let token = sign_token(&enc, &claims);

    let req = axum::http::Request::builder()
        .uri("/api/whoami")
        .header("authorization", bearer(&token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// Test AuthKit::from_parts and the convenience methods.
#[tokio::test]
async fn auth_kit_from_parts_provides_layers() {
    let (enc, pub_pem) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let auth = AuthKit::from_parts(verifier, "http://localhost:9999".into());

    let app = Router::new()
        .route("/api/whoami", get(whoami))
        .route_layer(auth.require_permissions(&["orders.read"]))
        .merge(auth.proxy_routes())
        .layer(auth.claims_layer());

    // Valid JWT → 200
    let claims = default_claims(vec!["orders.read".into()]);
    let token = sign_token(&enc, &claims);

    let req = axum::http::Request::builder()
        .uri("/api/whoami")
        .header("authorization", bearer(&token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // No JWT → 401
    let req = axum::http::Request::builder()
        .uri("/api/whoami")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Test the strict claims layer rejects unauthenticated requests.
#[tokio::test]
async fn strict_layer_rejects_all_unauthenticated() {
    let (_, pub_pem) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let auth = AuthKit::from_parts(verifier, "http://localhost:9999".into());

    let app = Router::new()
        .route("/api/public", get(|| async { "ok" }))
        .layer(auth.strict_claims_layer());

    let req = axum::http::Request::builder()
        .uri("/api/public")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Test that proxy routes exist (login/refresh/logout).
/// Without a real identity-auth, the proxy returns 502.
#[tokio::test]
async fn proxy_routes_return_502_when_identity_unreachable() {
    let (_, pub_pem) = make_keys();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let auth = AuthKit::from_parts(verifier, "http://127.0.0.1:1".into());
    let app = auth.proxy_routes();

    let body = serde_json::json!({
        "tenant_id": Uuid::new_v4(),
        "email": "test@example.com",
        "password": "secret"
    });

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}
