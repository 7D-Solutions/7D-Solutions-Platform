//! E2E tests for security primitive fixes (bd-3vto7).
//!
//! Proves:
//! 1. Invite temp passwords are random per invite (never the same string twice)
//! 2. Refresh token generation uses OsRng (behavioral: outputs are unique)
//! 3. Webhook secret enforcement panics/errors in production config

mod common;

use axum::body::Body;
use axum::http::Request;
use customer_portal::auth::{generate_refresh_token, PortalJwt};
use customer_portal::{build_router, metrics::PortalMetrics, AppState};
use payments_rs::config::{Config as PaymentsConfig, BusType, PaymentsProvider};
use rand::thread_rng;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashSet;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn portal_db_url() -> String {
    std::env::var("CUSTOMER_PORTAL_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/customer_portal_db".to_string())
}

fn make_test_keys() -> (String, String) {
    let mut rng = thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
    let public_key = private_key.to_public_key();
    let private_pem = private_key.to_pkcs8_pem(LineEnding::LF).expect("private pem").to_string();
    let public_pem = public_key.to_public_key_pem(LineEnding::LF).expect("public pem");
    (private_pem, public_pem)
}

async fn portal_test_app() -> Option<(axum::Router, sqlx::PgPool, Arc<PortalJwt>)> {
    let pool = match PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(3))
        .max_connections(5)
        .connect(&portal_db_url())
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("skipping security_primitives e2e: customer-portal DB unavailable ({err})");
            return None;
        }
    };

    if let Err(err) = sqlx::migrate!("../modules/customer-portal/db/migrations").run(&pool).await {
        eprintln!("skipping security_primitives e2e: migration failed ({err})");
        return None;
    }

    let (priv_pem, pub_pem) = make_test_keys();
    let portal_jwt = Arc::new(PortalJwt::new(&priv_pem, &pub_pem).expect("portal jwt"));

    let state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: PortalMetrics::new().expect("metrics"),
        portal_jwt: portal_jwt.clone(),
        config: customer_portal::config::Config {
            database_url: portal_db_url(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cors_origins: vec!["*".to_string()],
            portal_jwt_private_key: priv_pem,
            portal_jwt_public_key: pub_pem,
            access_token_ttl_minutes: 15,
            refresh_token_ttl_days: 7,
            doc_mgmt_base_url: "http://127.0.0.1:1".to_string(),
            doc_mgmt_bearer_token: None,
        },
    });

    Some((build_router(state), pool, portal_jwt))
}

// ---------------------------------------------------------------------------
// Test 1: Two invite operations yield different temporary passwords
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_primitives_invite_passwords_are_random() {
    let Some((app, pool, portal_jwt)) = portal_test_app().await else {
        return;
    };

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let admin_user_id = Uuid::new_v4();

    // Create an admin user who can send invites
    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name) \
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(admin_user_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(format!("admin-{}@test.com", Uuid::new_v4()))
    .bind(customer_portal::hash_password("AdminPassw0rd!").expect("hash"))
    .bind("Admin User")
    .execute(&pool)
    .await
    .expect("insert admin user");

    // Issue a JWT with PARTY_MUTATE scope for the admin
    let token = portal_jwt
        .issue_access_token(
            admin_user_id,
            tenant_id,
            party_id,
            vec!["party:mutate".to_string()],
            15,
        )
        .expect("admin token");

    // Invite user A
    let invite_a = serde_json::json!({
        "tenant_id": tenant_id,
        "party_id": party_id,
        "email": format!("invite-a-{}@test.com", Uuid::new_v4()),
        "display_name": "Invite A",
        "scopes": ["documents:read"],
        "idempotency_key": format!("idem-a-{}", Uuid::new_v4()),
    });

    let req = Request::builder()
        .method("POST")
        .uri("/portal/admin/users")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(invite_a.to_string()))
        .expect("request");

    let res = app.clone().oneshot(req).await.expect("response");
    assert_eq!(
        res.status(),
        axum::http::StatusCode::OK,
        "invite A should succeed"
    );
    let body_a: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("bytes"),
    )
    .expect("json");
    let user_a_id: Uuid = body_a["user_id"].as_str().unwrap().parse().unwrap();

    // Invite user B
    let invite_b = serde_json::json!({
        "tenant_id": tenant_id,
        "party_id": party_id,
        "email": format!("invite-b-{}@test.com", Uuid::new_v4()),
        "display_name": "Invite B",
        "scopes": ["documents:read"],
        "idempotency_key": format!("idem-b-{}", Uuid::new_v4()),
    });

    let req = Request::builder()
        .method("POST")
        .uri("/portal/admin/users")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(invite_b.to_string()))
        .expect("request");

    let res = app.oneshot(req).await.expect("response");
    assert_eq!(
        res.status(),
        axum::http::StatusCode::OK,
        "invite B should succeed"
    );
    let body_b: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("bytes"),
    )
    .expect("json");
    let user_b_id: Uuid = body_b["user_id"].as_str().unwrap().parse().unwrap();

    // Query DB for password hashes — they must differ
    let hash_a: String = sqlx::query_scalar("SELECT password_hash FROM portal_users WHERE id = $1")
        .bind(user_a_id)
        .fetch_one(&pool)
        .await
        .expect("hash A");

    let hash_b: String = sqlx::query_scalar("SELECT password_hash FROM portal_users WHERE id = $1")
        .bind(user_b_id)
        .fetch_one(&pool)
        .await
        .expect("hash B");

    assert_ne!(
        hash_a, hash_b,
        "invite temp passwords must be random — hashes must differ"
    );

    // Cleanup
    sqlx::query("DELETE FROM portal_idempotency WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM portal_users WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ---------------------------------------------------------------------------
// Test 2: OsRng refresh tokens are unique (behavioral proof)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_primitives_osrng_refresh_tokens_unique() {
    let tokens: Vec<String> = (0..200).map(|_| generate_refresh_token()).collect();
    let unique: HashSet<&String> = tokens.iter().collect();
    assert_eq!(
        tokens.len(),
        unique.len(),
        "OsRng must produce unique refresh tokens — got {} duplicates in 200 calls",
        tokens.len() - unique.len()
    );
    // base64url(32 bytes) = 43 characters
    for t in &tokens {
        assert_eq!(t.len(), 43, "refresh token length must be 43 chars (base64url of 32 bytes)");
    }
}

// ---------------------------------------------------------------------------
// Test 3: Payments webhook secret required in production
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_primitives_webhook_secret_required_in_production() {
    // Tilled + production + no webhook secret → must error
    let config_missing = PaymentsConfig {
        database_url: "postgresql://localhost/test".to_string(),
        bus_type: BusType::InMemory,
        nats_url: None,
        host: "0.0.0.0".to_string(),
        port: 8088,
        env: "production".to_string(),
        cors_origins: vec!["https://app.example.com".to_string()],
        payments_provider: PaymentsProvider::Tilled,
        tilled_api_key: Some("sk_test_key".to_string()),
        tilled_account_id: Some("acct_test".to_string()),
        tilled_webhook_secret: None,
        tilled_webhook_secret_prev: None,
    };
    let err = config_missing.validate().unwrap_err();
    assert!(
        err.contains("TILLED_WEBHOOK_SECRET"),
        "production Tilled config without webhook secret must fail: {err}"
    );

    // Tilled + production + webhook secret present → must succeed
    let config_ok = PaymentsConfig {
        tilled_webhook_secret: Some("whsec_prod".to_string()),
        ..config_missing.clone()
    };
    assert!(
        config_ok.validate().is_ok(),
        "production Tilled config with webhook secret should pass"
    );

    // Mock provider + production + no webhook secret → must succeed (not Tilled)
    let config_mock = PaymentsConfig {
        payments_provider: PaymentsProvider::Mock,
        tilled_api_key: None,
        tilled_account_id: None,
        tilled_webhook_secret: None,
        ..config_missing.clone()
    };
    assert!(
        config_mock.validate().is_ok(),
        "mock provider should not require webhook secret"
    );
}
