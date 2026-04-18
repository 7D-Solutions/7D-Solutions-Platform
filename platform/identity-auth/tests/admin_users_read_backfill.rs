//! Integration test for backfilling admin.users.read onto existing admin roles.
//!
//! Verifies the backfill migration restores access for legacy admin roles and
//! remains idempotent when re-applied.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::sync::Arc;
use tower::util::ServiceExt;
use uuid::Uuid;

use auth_rs::auth::{
    concurrency::HashConcurrencyLimiter, handlers::AuthState, jwt::JwtKeys,
    password::PasswordPolicy,
};
use auth_rs::clients::tenant_registry::TenantRegistryClient;
use auth_rs::events::publisher::EventPublisher;
use auth_rs::metrics::Metrics;
use auth_rs::rate_limit::KeyedLimiters;
use auth_rs::routes::auth::router;

fn test_db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into())
}

fn nats_url() -> String {
    std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://platform:dev-nats-token@localhost:4222".into())
}

async fn test_pool() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect to test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

fn generate_rsa_keys() -> (String, String) {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key generation");
    let pub_key = priv_key.to_public_key();
    let private_pem = priv_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("private PEM")
        .to_string();
    let public_pem = pub_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public PEM");
    (private_pem, public_pem)
}

async fn build_state(pool: PgPool) -> (Arc<AuthState>, JwtKeys) {
    let nats = event_bus::connect_nats(&nats_url())
        .await
        .expect("connect nats");
    let (private_pem, public_pem) = generate_rsa_keys();
    let jwt = JwtKeys::from_pem(&private_pem, &public_pem, "auth-key-test".to_string())
        .expect("jwt keys");

    let state = Arc::new(AuthState {
        db: pool,
        jwt: jwt.clone(),
        pwd: PasswordPolicy {
            memory_kb: 65_536,
            iterations: 3,
            parallelism: 1,
        },
        access_ttl_minutes: 15,
        refresh_ttl_days: 14,
        refresh_idle_minutes: 480,
        refresh_absolute_max_days: 30,
        cookie_secure: false,
        events: EventPublisher::new(nats),
        producer: format!("auth-rs@{}", env!("CARGO_PKG_VERSION")),
        metrics: Metrics::new(),
        keyed_limits: KeyedLimiters::new(),
        hash_limiter: HashConcurrencyLimiter::new(50, 5_000),
        lockout_threshold: 10,
        lockout_minutes: 15,
        login_per_min_per_email: 5,
        register_per_min_per_email: 5,
        refresh_per_min_per_token: 20,
        forgot_per_min_per_email: 3,
        forgot_per_min_per_ip: 10,
        reset_per_min_per_ip: 5,
        password_reset_ttl_minutes: 30,
        max_concurrent_sessions: 5,
        tenant_registry: None::<TenantRegistryClient>,
    });

    (state, jwt)
}

async fn insert_credential(pool: &PgPool, tenant_id: Uuid, user_id: Uuid, email: &str) {
    sqlx::query(
        r#"INSERT INTO credentials (tenant_id, user_id, email, password_hash, is_active)
           VALUES ($1, $2, $3, $4, TRUE)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(email)
    .bind("test-hash-not-real")
    .execute(pool)
    .await
    .expect("insert credential");
}

async fn bind_admin_role(pool: &PgPool, tenant_id: Uuid, user_id: Uuid) -> Uuid {
    let role: auth_rs::db::rbac::Role =
        auth_rs::db::rbac::create_role(pool, tenant_id, "admin", "Tenant admin", true)
            .await
            .expect("create system admin role");

    sqlx::query("INSERT INTO user_role_bindings (tenant_id, user_id, role_id) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind(user_id)
        .bind(role.id)
        .execute(pool)
        .await
        .expect("bind admin role");

    role.id
}

async fn count_admin_grants(pool: &PgPool, role_id: Uuid) -> i64 {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM role_permissions rp
        JOIN permissions p ON p.id = rp.permission_id
        WHERE rp.role_id = $1 AND p.key = 'admin.users.read'
        "#,
    )
    .bind(role_id)
    .fetch_one(pool)
    .await
    .expect("count grants")
}

async fn call_admin_users(
    state: Arc<AuthState>,
    token: String,
    tenant_id: Uuid,
) -> (StatusCode, String) {
    let app: axum::Router = router(state);
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/auth/admin/users?tenant_id={tenant_id}&include_inactive=false"
        ))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("router call");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = String::from_utf8_lossy(&body).into_owned();
    (status, body)
}

#[tokio::test]
async fn backfill_admin_users_read_restores_endpoint_access_and_is_idempotent() {
    let pool = test_pool().await;
    let (state, jwt) = build_state(pool.clone()).await;

    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("legacy-admin-{}@example.com", &user_id.to_string()[..8]);

    insert_credential(&pool, tenant_id, user_id, &email).await;
    let role_id = bind_admin_role(&pool, tenant_id, user_id).await;

    let token = jwt
        .sign_access_token(
            tenant_id,
            user_id,
            vec!["admin".to_string()],
            vec![],
            auth_rs::auth::jwt::actor_type::USER,
            15,
        )
        .expect("mint token");

    let (before_status, before_json) =
        call_admin_users(state.clone(), token.clone(), tenant_id).await;
    assert_eq!(before_status, StatusCode::FORBIDDEN);
    assert_eq!(before_json, "insufficient permissions");
    assert_eq!(count_admin_grants(&pool, role_id).await, 0);

    sqlx::query(include_str!(
        "../db/migrations/011_backfill_admin_users_read.sql"
    ))
    .execute(&pool)
    .await
    .expect("run backfill migration");

    assert_eq!(count_admin_grants(&pool, role_id).await, 1);

    let (after_status, after_json) =
        call_admin_users(state.clone(), token.clone(), tenant_id).await;
    assert_eq!(after_status, StatusCode::OK);
    let users: serde_json::Value = serde_json::from_str(&after_json).expect("parse json");
    let users = users.as_array().expect("users array");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["email"], email);
    assert!(users[0]["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .any(|p| p == "admin.users.read"));

    sqlx::query(include_str!(
        "../db/migrations/011_backfill_admin_users_read.sql"
    ))
    .execute(&pool)
    .await
    .expect("re-run backfill migration");

    assert_eq!(count_admin_grants(&pool, role_id).await, 1);

    let (second_status, second_json) = call_admin_users(state, token, tenant_id).await;
    assert_eq!(second_status, StatusCode::OK);
    let second_users: serde_json::Value = serde_json::from_str(&second_json).expect("parse json");
    assert_eq!(second_users.as_array().expect("users array").len(), 1);

    sqlx::query("DELETE FROM user_role_bindings WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM role_permissions WHERE role_id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM credentials WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
}
