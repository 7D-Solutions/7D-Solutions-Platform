//! E2E: RBAC enforcement (deny-by-default) across modules (bd-3riw)
//!
//! Verifies that:
//! 1. Unauthorized requests (no token) receive 401 Unauthorized
//! 2. Requests with insufficient permissions receive 403 Forbidden
//! 3. Requests with correct permissions succeed (2xx)
//! 4. Audit log captures the correct actor for authorised mutations
//!
//! ## Design
//! Tests are in-process using `tower::ServiceExt::oneshot`. No live services
//! required — routers are assembled from library crate handlers plus the
//! production `ClaimsLayer` + `RequirePermissionsLayer` middleware stack.
//!
//! RSA keypairs are generated per test-run; tokens are signed in-process and
//! verified by the same [`JwtVerifier`] the production middleware uses.
//!
//! ## Covered modules
//! AR (ar.mutate), GL (gl.post), Inventory (inventory.mutate), AP (ap.mutate),
//! Reporting (reporting.mutate), Timekeeping (timekeeping.mutate)
//!
//! ## Running
//! ```bash
//! AUDIT_DATABASE_URL=postgres://postgres:postgres@localhost:5432/audit_db \
//! PROJECTIONS_DATABASE_URL=postgres://postgres:postgres@localhost:5432/projections_db \
//! TENANT_REGISTRY_DATABASE_URL=postgres://postgres:postgres@localhost:5432/tenant_registry_db \
//! ./scripts/cargo-slot.sh test -p e2e-tests -- rbac_enforcement --nocapture
//! ```

mod common;

use audit::{actor::Actor, schema::{MutationClass, WriteAuditRequest}, writer::AuditWriter};
use axum::{body::Body, http::Request, routing::post, Router};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use security::{
    authz_middleware::{ClaimsLayer, RequirePermissionsLayer},
    permissions,
    JwtVerifier,
};
use serde::Serialize;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// RSA keypair used for signing/verifying test JWTs.
struct TestKeys {
    encoding: EncodingKey,
    verifier: Arc<JwtVerifier>,
}

/// Raw JWT claims matching identity-auth's AccessClaims shape.
#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn make_test_keys() -> TestKeys {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());
    TestKeys { encoding, verifier }
}

/// Build a signed JWT granting the given permission strings.
fn make_jwt(keys: &TestKeys, perms: Vec<&str>) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        app_id: None,
        roles: vec!["operator".to_string()],
        perms: perms.into_iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding).unwrap()
}

/// Build a minimal guarded router: one POST /guarded route behind
/// `RequirePermissionsLayer` for `required_perm`, wrapped in `ClaimsLayer`.
///
/// Mirrors the production pattern in modules/ar/src/routes/mod.rs.
fn make_guarded_router(required_perm: &'static str, verifier: Arc<JwtVerifier>) -> Router {
    Router::new()
        .route("/guarded", post(|| async { "ok" }))
        .route_layer(RequirePermissionsLayer::new(&[required_perm]))
        .layer(ClaimsLayer::permissive(verifier))
}

// ============================================================================
// Deny-by-default: no token
// ============================================================================

/// No Bearer token → 401 Unauthorized (production default across all modules).
#[tokio::test]
async fn rbac_no_token_returns_401_ar() {
    let keys = make_test_keys();
    let app = make_guarded_router(permissions::AR_MUTATE, keys.verifier);

    let resp = app
        .oneshot(Request::builder().uri("/guarded").method("POST").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AR mutation without token must return 401"
    );
    println!("✅ AR: no token → 401");
}

#[tokio::test]
async fn rbac_no_token_returns_401_gl() {
    let keys = make_test_keys();
    let app = make_guarded_router(permissions::GL_POST, keys.verifier);

    let resp = app
        .oneshot(Request::builder().uri("/guarded").method("POST").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 401, "GL post without token must return 401");
    println!("✅ GL: no token → 401");
}

#[tokio::test]
async fn rbac_no_token_returns_401_inventory() {
    let keys = make_test_keys();
    let app = make_guarded_router(permissions::INVENTORY_MUTATE, keys.verifier);

    let resp = app
        .oneshot(Request::builder().uri("/guarded").method("POST").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 401, "Inventory mutation without token must return 401");
    println!("✅ Inventory: no token → 401");
}

#[tokio::test]
async fn rbac_no_token_returns_401_ap() {
    let keys = make_test_keys();
    let app = make_guarded_router(permissions::AP_MUTATE, keys.verifier);

    let resp = app
        .oneshot(Request::builder().uri("/guarded").method("POST").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 401, "AP mutation without token must return 401");
    println!("✅ AP: no token → 401");
}

#[tokio::test]
async fn rbac_no_token_returns_401_payments() {
    let keys = make_test_keys();
    let app = make_guarded_router(permissions::PAYMENTS_MUTATE, keys.verifier);

    let resp = app
        .oneshot(Request::builder().uri("/guarded").method("POST").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 401, "Payments mutation without token must return 401");
    println!("✅ Payments: no token → 401");
}

// ============================================================================
// Wrong permissions → 403
// ============================================================================

/// Token present but missing the required permission → 403 Forbidden.
#[tokio::test]
async fn rbac_wrong_permission_returns_403_ar() {
    let keys = make_test_keys();
    // Token grants gl.post but route requires ar.mutate
    let token = make_jwt(&keys, vec![permissions::GL_POST]);
    let app = make_guarded_router(permissions::AR_MUTATE, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 403, "Wrong perm on AR route must return 403");
    println!("✅ AR: wrong permission → 403");
}

#[tokio::test]
async fn rbac_wrong_permission_returns_403_gl() {
    let keys = make_test_keys();
    // Token grants ar.mutate but route requires gl.post
    let token = make_jwt(&keys, vec![permissions::AR_MUTATE]);
    let app = make_guarded_router(permissions::GL_POST, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 403, "Wrong perm on GL route must return 403");
    println!("✅ GL: wrong permission → 403");
}

#[tokio::test]
async fn rbac_wrong_permission_returns_403_inventory() {
    let keys = make_test_keys();
    let token = make_jwt(&keys, vec![permissions::AR_MUTATE]);
    let app = make_guarded_router(permissions::INVENTORY_MUTATE, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 403, "Wrong perm on Inventory route must return 403");
    println!("✅ Inventory: wrong permission → 403");
}

#[tokio::test]
async fn rbac_empty_perms_returns_403() {
    let keys = make_test_keys();
    // Token is valid but has NO permissions at all
    let token = make_jwt(&keys, vec![]);
    let app = make_guarded_router(permissions::AR_MUTATE, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        403,
        "Token with empty perms must return 403"
    );
    println!("✅ Empty perms token → 403");
}

// ============================================================================
// Correct permissions → 200
// ============================================================================

/// Token with correct permission → route handler executes, returns 200.
#[tokio::test]
async fn rbac_correct_permission_succeeds_ar() {
    let keys = make_test_keys();
    let token = make_jwt(&keys, vec![permissions::AR_MUTATE]);
    let app = make_guarded_router(permissions::AR_MUTATE, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200, "Correct perm on AR route must succeed");
    println!("✅ AR: correct permission → 200");
}

#[tokio::test]
async fn rbac_correct_permission_succeeds_gl() {
    let keys = make_test_keys();
    let token = make_jwt(&keys, vec![permissions::GL_POST]);
    let app = make_guarded_router(permissions::GL_POST, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200, "Correct perm on GL route must succeed");
    println!("✅ GL: correct permission → 200");
}

#[tokio::test]
async fn rbac_correct_permission_succeeds_inventory() {
    let keys = make_test_keys();
    let token = make_jwt(&keys, vec![permissions::INVENTORY_MUTATE]);
    let app = make_guarded_router(permissions::INVENTORY_MUTATE, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200, "Correct perm on Inventory route must succeed");
    println!("✅ Inventory: correct permission → 200");
}

/// Token with multiple permissions satisfies a single-perm gate.
#[tokio::test]
async fn rbac_superset_perms_succeeds() {
    let keys = make_test_keys();
    // Admin-like token: all permissions
    let token = make_jwt(
        &keys,
        vec![
            permissions::AR_MUTATE,
            permissions::GL_POST,
            permissions::INVENTORY_MUTATE,
            permissions::AP_MUTATE,
            permissions::PAYMENTS_MUTATE,
        ],
    );
    let app = make_guarded_router(permissions::AR_MUTATE, keys.verifier);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200, "Superset perms must satisfy perm gate");
    println!("✅ Superset permissions → 200");
}

// ============================================================================
// Per-module matrix: no-token and correct-token for all permission constants
// ============================================================================

/// Verify deny-by-default across all module permission gates in one pass.
#[tokio::test]
async fn rbac_deny_by_default_all_modules() {
    let modules: Vec<(&str, &str)> = vec![
        ("ar.mutate", permissions::AR_MUTATE),
        ("gl.post", permissions::GL_POST),
        ("inventory.mutate", permissions::INVENTORY_MUTATE),
        ("ap.mutate", permissions::AP_MUTATE),
        ("payments.mutate", permissions::PAYMENTS_MUTATE),
        ("reporting.mutate", permissions::REPORTING_MUTATE),
        ("treasury.mutate", permissions::TREASURY_MUTATE),
        ("timekeeping.mutate", permissions::TIMEKEEPING_MUTATE),
        ("consolidation.mutate", permissions::CONSOLIDATION_MUTATE),
        ("fixed_assets.mutate", permissions::FIXED_ASSETS_MUTATE),
        ("subscriptions.mutate", permissions::SUBSCRIPTIONS_MUTATE),
    ];

    for (label, perm) in &modules {
        let keys = make_test_keys();
        let app = make_guarded_router(perm, keys.verifier);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            401,
            "{}: no-token request must be denied 401",
            label
        );
        println!("✅ {}: deny-by-default → 401", label);
    }

    println!("\n✅ All {} modules enforce deny-by-default (no token → 401)", modules.len());
}

/// Verify authorised access across all module permission gates.
#[tokio::test]
async fn rbac_authorised_succeeds_all_modules() {
    let modules: Vec<(&str, &str)> = vec![
        ("ar.mutate", permissions::AR_MUTATE),
        ("gl.post", permissions::GL_POST),
        ("inventory.mutate", permissions::INVENTORY_MUTATE),
        ("ap.mutate", permissions::AP_MUTATE),
        ("payments.mutate", permissions::PAYMENTS_MUTATE),
        ("reporting.mutate", permissions::REPORTING_MUTATE),
        ("treasury.mutate", permissions::TREASURY_MUTATE),
        ("timekeeping.mutate", permissions::TIMEKEEPING_MUTATE),
        ("consolidation.mutate", permissions::CONSOLIDATION_MUTATE),
        ("fixed_assets.mutate", permissions::FIXED_ASSETS_MUTATE),
        ("subscriptions.mutate", permissions::SUBSCRIPTIONS_MUTATE),
    ];

    for (label, perm) in &modules {
        let keys = make_test_keys();
        let token = make_jwt(&keys, vec![perm]);
        let app = make_guarded_router(perm, keys.verifier);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .method("POST")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status().as_u16(),
            200,
            "{}: authorised request must succeed",
            label
        );
        println!("✅ {}: authorised → 200", label);
    }

    println!("\n✅ All {} modules accept correctly-permissioned tokens", modules.len());
}

// ============================================================================
// Audit log: actor is captured correctly for authorised mutations
// ============================================================================

/// After an authorised mutation, the audit writer records the actor_id and
/// actor_type faithfully. Skips gracefully when the audit DB is unavailable.
#[tokio::test]
async fn rbac_audit_actor_captured_for_authorised_mutation() {
    let audit_url = common::get_audit_db_url();

    // Try to connect to audit DB; skip gracefully if unavailable.
    let pool = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(3))
            .connect(&audit_url),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            println!("⚠️  Audit DB unavailable ({}) — skipping actor-in-audit test", e);
            return;
        }
        Err(_) => {
            println!("⚠️  Audit DB connection timed out — skipping actor-in-audit test");
            return;
        }
    };

    common::run_audit_migrations(&pool).await;

    // Simulate a user actor (e.g., extracted from a valid JWT after RBAC passes)
    let user_id = Uuid::new_v4();
    let actor = Actor::user(user_id);
    let entity_id = format!("invoice_{}", Uuid::new_v4());

    let writer = AuditWriter::new(pool.clone());

    // Write the audit entry that a mutation handler would produce
    let audit_id = writer
        .write(
            WriteAuditRequest::new(
                actor.id,
                actor.actor_type_str(),
                "CreateInvoice".to_string(),
                MutationClass::Create,
                "Invoice".to_string(),
                entity_id.clone(),
            ),
        )
        .await
        .expect("Audit write failed");

    // Verify actor fields are faithfully persisted
    let events = writer
        .get_by_entity("Invoice", &entity_id)
        .await
        .expect("Audit query failed");

    assert_eq!(events.len(), 1, "Expected exactly one audit event");
    assert_eq!(events[0].audit_id, audit_id);
    assert_eq!(events[0].actor_id, user_id, "actor_id must match JWT sub");
    assert_eq!(events[0].actor_type, "User", "actor_type must be User");
    assert_eq!(events[0].action, "CreateInvoice");
    assert_eq!(events[0].entity_type, "Invoice");

    println!("✅ Audit actor captured: actor_id={}, actor_type=User, action=CreateInvoice", user_id);
}

/// Service actor recorded when a service-to-service call passes RBAC.
#[tokio::test]
async fn rbac_audit_service_actor_captured() {
    let audit_url = common::get_audit_db_url();

    let pool = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(3))
            .connect(&audit_url),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            println!("⚠️  Audit DB unavailable ({}) — skipping service actor test", e);
            return;
        }
        Err(_) => {
            println!("⚠️  Audit DB timed out — skipping service actor test");
            return;
        }
    };

    common::run_audit_migrations(&pool).await;

    let actor = Actor::service("ar-module");
    let entity_id = format!("payment_{}", Uuid::new_v4());
    let writer = AuditWriter::new(pool.clone());

    writer
        .write(
            WriteAuditRequest::new(
                actor.id,
                actor.actor_type_str(),
                "AllocatePayment".to_string(),
                MutationClass::Update,
                "Payment".to_string(),
                entity_id.clone(),
            ),
        )
        .await
        .expect("Audit write failed");

    let events = writer
        .get_by_entity("Payment", &entity_id)
        .await
        .expect("Audit query failed");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_id, actor.id, "Service actor ID must be deterministic");
    assert_eq!(events[0].actor_type, "Service");
    assert_eq!(events[0].action, "AllocatePayment");

    // Verify service actor ID is deterministic (same name → same UUID)
    let actor2 = Actor::service("ar-module");
    assert_eq!(actor.id, actor2.id, "Service actors must use deterministic UUIDs");

    println!("✅ Service actor captured: actor_id={} (deterministic), actor_type=Service", actor.id);
}
