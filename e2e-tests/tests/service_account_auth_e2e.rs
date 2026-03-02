//! E2E: service-to-service auth with app_id — prove actor_type=service JWT works
//! for AR + Party calls (bd-kxym).
//!
//! Verifies that machine-to-machine authentication works correctly for the
//! TrashTech integration pattern. Proves:
//!
//! 1. actor_type=service JWTs are correctly extracted by ClaimsLayer
//! 2. VerifiedClaims.actor_type is ActorType::Service (not User)
//! 3. Service JWTs with correct permissions pass AR mutation routes
//! 4. Service JWTs without permissions receive 403 Forbidden
//! 5. Missing token receives 401 Unauthorized (deny-by-default)
//! 6. Audit trail records actor_type=Service for service actors
//! 7. User JWTs retain actor_type=User — no actor type confusion
//!
//! ## Design
//! Tests run in-process via `tower::ServiceExt::oneshot`. No live services needed.
//! RSA keypairs are generated per test-run; tokens are signed locally and verified
//! by the same [`JwtVerifier`] the production ClaimsLayer uses.
//!
//! ## Invariant
//! Service tokens must be scoped with explicit permissions (same model as user tokens).
//! The `actor_type` field in the JWT is immutable once signed — there is no upgrade
//! path from `user` to `service`. Audit trail always records the correct actor_type.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- service_account_auth_e2e --nocapture
//! ```

mod common;

use audit::{
    actor::Actor,
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use axum::{body::Body, http::Request, routing::post, Router};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use security::{
    authz_middleware::{ClaimsLayer, RequirePermissionsLayer},
    claims::ActorType,
    permissions, JwtVerifier,
};
use serde::Serialize;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Test infrastructure
// ============================================================================

/// RSA keypair for signing/verifying test JWTs.
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
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Sign a JWT with the given actor_type, permissions, and optional app_id.
fn sign_jwt(
    keys: &TestKeys,
    actor_type: &str,
    user_id: Option<Uuid>,
    perms: Vec<&str>,
    app_id: Option<Uuid>,
) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: user_id.unwrap_or_else(Uuid::new_v4).to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        app_id: app_id.map(|id| id.to_string()),
        roles: vec!["service-account".to_string()],
        perms: perms.into_iter().map(|s| s.to_string()).collect(),
        actor_type: actor_type.to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding).unwrap()
}

/// Build a guarded router requiring the given permission (mirrors production AR pattern).
fn make_guarded_router(required_perm: &'static str, verifier: Arc<JwtVerifier>) -> Router {
    Router::new()
        .route("/guarded", post(|| async { "ok" }))
        .route_layer(RequirePermissionsLayer::new(&[required_perm]))
        .layer(ClaimsLayer::permissive(verifier))
}

// ============================================================================
// Tests: JWT claims extraction — actor_type=service is correctly parsed
// ============================================================================

/// Service JWT correctly produces ActorType::Service in VerifiedClaims.
#[tokio::test]
async fn service_jwt_actor_type_is_service() {
    let keys = make_test_keys();
    let token = sign_jwt(&keys, "service", None, vec![permissions::AR_MUTATE], None);

    let claims = keys
        .verifier
        .verify(&token)
        .expect("Token verification failed");

    assert_eq!(
        claims.actor_type,
        ActorType::Service,
        "Service JWT must produce actor_type=Service in VerifiedClaims"
    );
    println!("✅ actor_type=service JWT → VerifiedClaims.actor_type = Service");
}

/// User JWT correctly produces ActorType::User — no type confusion with service.
#[tokio::test]
async fn user_jwt_actor_type_is_user() {
    let keys = make_test_keys();
    let token = sign_jwt(&keys, "user", None, vec![permissions::AR_MUTATE], None);

    let claims = keys
        .verifier
        .verify(&token)
        .expect("Token verification failed");

    assert_eq!(
        claims.actor_type,
        ActorType::User,
        "User JWT must produce actor_type=User in VerifiedClaims"
    );
    println!("✅ actor_type=user JWT → VerifiedClaims.actor_type = User");
}

/// System JWT correctly produces ActorType::System.
#[tokio::test]
async fn system_jwt_actor_type_is_system() {
    let keys = make_test_keys();
    let token = sign_jwt(&keys, "system", None, vec![], None);

    let claims = keys
        .verifier
        .verify(&token)
        .expect("Token verification failed");

    assert_eq!(
        claims.actor_type,
        ActorType::System,
        "System JWT must produce actor_type=System in VerifiedClaims"
    );
    println!("✅ actor_type=system JWT → VerifiedClaims.actor_type = System");
}

/// app_id is carried through correctly in a service JWT (TrashTech integration pattern).
#[tokio::test]
async fn service_jwt_app_id_preserved() {
    let keys = make_test_keys();
    let app_id = Uuid::new_v4();
    let service_machine_id = Uuid::new_v4();

    let token = sign_jwt(
        &keys,
        "service",
        Some(service_machine_id),
        vec![permissions::AR_MUTATE],
        Some(app_id),
    );

    let claims = keys
        .verifier
        .verify(&token)
        .expect("Token verification failed");

    assert_eq!(
        claims.actor_type,
        ActorType::Service,
        "Service JWT must produce actor_type=Service"
    );
    assert_eq!(
        claims.app_id,
        Some(app_id),
        "app_id must be preserved in VerifiedClaims"
    );
    assert_eq!(
        claims.user_id, service_machine_id,
        "service machine UUID must be preserved as user_id (sub)"
    );
    println!(
        "✅ Service JWT with app_id: actor_type=Service, app_id={}, machine_id={}",
        app_id, service_machine_id
    );
}

/// Service and user JWTs have different actor_type — no cross-type confusion.
#[tokio::test]
async fn service_and_user_actor_types_are_distinct() {
    let keys = make_test_keys();

    let svc_token = sign_jwt(&keys, "service", None, vec![permissions::AR_MUTATE], None);
    let usr_token = sign_jwt(&keys, "user", None, vec![permissions::AR_MUTATE], None);

    let svc_claims = keys.verifier.verify(&svc_token).unwrap();
    let usr_claims = keys.verifier.verify(&usr_token).unwrap();

    assert_eq!(svc_claims.actor_type, ActorType::Service);
    assert_eq!(usr_claims.actor_type, ActorType::User);
    assert_ne!(
        svc_claims.actor_type, usr_claims.actor_type,
        "Service and user actor types must differ"
    );
    println!("✅ Service vs User actor types are distinct — no type confusion");
}

// ============================================================================
// Tests: AR route access with service JWT
// ============================================================================

/// Service JWT with ar.mutate permission succeeds on AR mutation route.
#[tokio::test]
async fn service_jwt_with_ar_mutate_perm_succeeds() {
    let keys = make_test_keys();
    let token = sign_jwt(&keys, "service", None, vec![permissions::AR_MUTATE], None);
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
        200,
        "Service JWT with ar.mutate must pass AR mutation route"
    );
    println!("✅ Service JWT + ar.mutate → AR route → 200 OK");
}

/// Service JWT without any permissions receives 403 Forbidden on AR route.
#[tokio::test]
async fn service_jwt_without_perms_gets_403_on_ar() {
    let keys = make_test_keys();
    let token = sign_jwt(&keys, "service", None, vec![], None); // No permissions
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
        "Service JWT without ar.mutate must be rejected with 403"
    );
    println!("✅ Service JWT without perms → AR route → 403 Forbidden");
}

/// Service JWT with wrong permission (gl.post instead of ar.mutate) receives 403.
#[tokio::test]
async fn service_jwt_wrong_perm_gets_403_on_ar() {
    let keys = make_test_keys();
    // GL service account trying to access AR
    let token = sign_jwt(&keys, "service", None, vec![permissions::GL_POST], None);
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
        "Service JWT with gl.post (wrong perm) on AR route must return 403"
    );
    println!("✅ Service JWT with gl.post → AR route → 403 Forbidden (wrong perm)");
}

/// No token at all receives 401 Unauthorized (deny-by-default).
#[tokio::test]
async fn no_token_gets_401_deny_by_default_ar() {
    let keys = make_test_keys();
    let app = make_guarded_router(permissions::AR_MUTATE, keys.verifier);

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
        "Missing token must return 401 (deny by default)"
    );
    println!("✅ No token → AR route → 401 Unauthorized");
}

/// Service JWT with superset permissions satisfies a single-perm gate on AR.
#[tokio::test]
async fn service_jwt_superset_perms_succeeds_ar() {
    let keys = make_test_keys();
    // Integration account with broad permissions
    let token = sign_jwt(
        &keys,
        "service",
        None,
        vec![
            permissions::AR_MUTATE,
            permissions::GL_POST,
            permissions::AP_MUTATE,
        ],
        None,
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

    assert_eq!(
        resp.status().as_u16(),
        200,
        "Service JWT with superset permissions must pass single-perm AR gate"
    );
    println!("✅ Service JWT with superset perms → AR route → 200 OK");
}

// ============================================================================
// Tests: Party route claims (Party module uses X-App-Id, not JWT auth middleware)
//
// Party does not enforce RequirePermissionsLayer. These tests verify that the
// ClaimsLayer correctly parses actor_type=service for Party-bound JWTs —
// proving the token is well-formed and usable by any service that reads claims.
// ============================================================================

/// Service JWT bound for Party calls carries correct actor_type in verified claims.
#[tokio::test]
async fn service_jwt_for_party_has_correct_actor_type() {
    let keys = make_test_keys();
    let app_id = Uuid::new_v4();
    let machine_id = Uuid::new_v4();

    let token = sign_jwt(
        &keys,
        "service",
        Some(machine_id),
        vec!["party.mutate"],
        Some(app_id),
    );

    let claims = keys.verifier.verify(&token).expect("Token must be valid");

    assert_eq!(claims.actor_type, ActorType::Service);
    assert_eq!(claims.app_id, Some(app_id));
    assert_eq!(claims.user_id, machine_id);
    assert!(
        claims.perms.contains(&"party.mutate".to_string()),
        "party.mutate must be in perms"
    );

    println!(
        "✅ Service JWT for Party: actor_type=Service, app_id={}, machine_id={}",
        app_id, machine_id
    );
}

// ============================================================================
// Tests: Audit trail — actor_type=service is correctly recorded
// ============================================================================

/// Service actor audit entry has actor_type=Service (against real audit DB).
///
/// Skips gracefully if the audit DB is unavailable.
#[tokio::test]
async fn service_actor_audit_records_actor_type_service() {
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
            println!(
                "⚠️  Audit DB unavailable ({}) — skipping service-actor audit test",
                e
            );
            return;
        }
        Err(_) => {
            println!("⚠️  Audit DB connection timed out — skipping service-actor audit test");
            return;
        }
    };

    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let entity_id = format!("svc_auth_e2e_{}", Uuid::new_v4());

    // Simulate TrashTech integration service calling AR
    let actor = Actor::service("trashtech-integration");

    let audit_id = writer
        .write(WriteAuditRequest::new(
            actor.id,
            actor.actor_type_str(),
            "CreateCustomer".to_string(),
            MutationClass::Create,
            "Customer".to_string(),
            entity_id.clone(),
        ))
        .await
        .expect("Audit write must succeed");

    let events = writer
        .get_by_entity("Customer", &entity_id)
        .await
        .expect("Audit query must succeed");

    assert_eq!(events.len(), 1, "Exactly one audit entry expected");
    assert_eq!(events[0].audit_id, audit_id);
    assert_eq!(events[0].actor_id, actor.id);
    assert_eq!(
        events[0].actor_type, "Service",
        "Audit actor_type must be 'Service'"
    );
    assert_eq!(events[0].action, "CreateCustomer");

    println!("✅ Audit trail: actor_type=Service recorded for CreateCustomer");
    println!("   actor_id={}, audit_id={}", actor.id, audit_id);
}

/// Service actor IDs are deterministic — same name → same UUID (reproducible audit trail).
#[tokio::test]
async fn service_actor_id_is_deterministic() {
    let actor1 = Actor::service("trashtech-integration");
    let actor2 = Actor::service("trashtech-integration");

    assert_eq!(
        actor1.id, actor2.id,
        "Service actor ID must be deterministic: same name → same UUID"
    );

    let actor3 = Actor::service("different-service");
    assert_ne!(
        actor1.id, actor3.id,
        "Different service names must produce different actor IDs"
    );

    println!("✅ Service actor ID is deterministic");
    println!("   trashtech-integration → {}", actor1.id);
    println!("   different-service     → {}", actor3.id);
}

/// Audit trail correctly distinguishes service actors from user actors.
///
/// Skips gracefully if the audit DB is unavailable.
#[tokio::test]
async fn audit_service_and_user_actors_not_confused() {
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
            println!(
                "⚠️  Audit DB unavailable ({}) — skipping actor-type confusion test",
                e
            );
            return;
        }
        Err(_) => {
            println!("⚠️  Audit DB connection timed out — skipping actor-type confusion test");
            return;
        }
    };

    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let user_id = Uuid::new_v4();
    let service_entity_id = format!("svc_e2e_{}", Uuid::new_v4());
    let user_entity_id = format!("usr_e2e_{}", Uuid::new_v4());

    // Write a service actor entry
    let service_actor = Actor::service("trashtech-integration");
    writer
        .write(WriteAuditRequest::new(
            service_actor.id,
            service_actor.actor_type_str(),
            "CreateCustomer".to_string(),
            MutationClass::Create,
            "Customer".to_string(),
            service_entity_id.clone(),
        ))
        .await
        .expect("Service audit write must succeed");

    // Write a user actor entry
    let user_actor = Actor::user(user_id);
    writer
        .write(WriteAuditRequest::new(
            user_actor.id,
            user_actor.actor_type_str(),
            "CreateCustomer".to_string(),
            MutationClass::Create,
            "Customer".to_string(),
            user_entity_id.clone(),
        ))
        .await
        .expect("User audit write must succeed");

    // Verify service entry
    let svc_events = writer
        .get_by_entity("Customer", &service_entity_id)
        .await
        .unwrap();
    assert_eq!(svc_events[0].actor_type, "Service");
    assert_eq!(svc_events[0].actor_id, service_actor.id);

    // Verify user entry
    let usr_events = writer
        .get_by_entity("Customer", &user_entity_id)
        .await
        .unwrap();
    assert_eq!(usr_events[0].actor_type, "User");
    assert_eq!(usr_events[0].actor_id, user_id);

    println!("✅ Audit trail: Service vs User actor types correctly distinguished");
    println!("   Service actor_type='Service', User actor_type='User'");
}

// ============================================================================
// Tests: Full invariant summary
// ============================================================================

/// All service-auth invariants hold in a single test.
///
/// Invariant: service tokens scoped to explicit permissions (same model as user tokens).
/// The actor_type field is immutable once signed — no upgrade from user to service.
#[tokio::test]
async fn service_auth_invariants_hold() {
    let keys = make_test_keys();

    // Invariant 1: service JWT with correct perm → 200 OK
    let svc_token = sign_jwt(&keys, "service", None, vec![permissions::AR_MUTATE], None);
    let app1 = make_guarded_router(permissions::AR_MUTATE, keys.verifier.clone());
    let resp1 = app1
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", svc_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp1.status().as_u16(),
        200,
        "Invariant 1: service+perm → 200"
    );

    // Invariant 2: service JWT without perm → 403
    let svc_noperm = sign_jwt(&keys, "service", None, vec![], None);
    let app2 = make_guarded_router(permissions::AR_MUTATE, keys.verifier.clone());
    let resp2 = app2
        .oneshot(
            Request::builder()
                .uri("/guarded")
                .method("POST")
                .header("Authorization", format!("Bearer {}", svc_noperm))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp2.status().as_u16(),
        403,
        "Invariant 2: service+no-perm → 403"
    );

    // Invariant 3: actor_type is immutable (no type coercion from JWT)
    let svc_claims = keys.verifier.verify(&svc_token).unwrap();
    let usr_token = sign_jwt(&keys, "user", None, vec![permissions::AR_MUTATE], None);
    let usr_claims = keys.verifier.verify(&usr_token).unwrap();
    assert_eq!(
        svc_claims.actor_type,
        ActorType::Service,
        "Invariant 3a: service token → Service"
    );
    assert_eq!(
        usr_claims.actor_type,
        ActorType::User,
        "Invariant 3b: user token → User"
    );
    assert_ne!(
        svc_claims.actor_type, usr_claims.actor_type,
        "Invariant 3c: types must differ — no coercion"
    );

    println!("✅ All service-auth invariants hold:");
    println!("   [1] service+perm → 200 ✓");
    println!("   [2] service+no-perm → 403 ✓");
    println!("   [3] actor_type immutable (Service ≠ User) ✓");
}
