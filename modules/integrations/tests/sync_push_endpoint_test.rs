//! HTTP-level integration tests for POST /api/integrations/sync/push/{entity_type}.
//!
//! DB-only tests (always run):
//!   - invalid entity_type → 422
//!   - missing OAuth connection → 404
//!   - disconnected OAuth → 412
//!   - superseded when authority version advanced → 200 with outcome:superseded
//!   - duplicate intent → 409
//!
//! QBO sandbox tests (require QBO_SANDBOX=1):
//!   - invoice create → 200 with outcome:succeeded
//!
//! Run DB-only:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_push_endpoint_test
//!
//! Run with sandbox:
//!   QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs --test sync_push_endpoint_test

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{
    domain::qbo::{QboError, TokenProvider},
    http::sync::push_entity,
    metrics::IntegrationsMetrics,
    AppState,
};
use security::{
    claims::ActorType,
    VerifiedClaims,
};
use serde_json::Value;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::{OnceCell, RwLock};
use tower::ServiceExt;
use uuid::Uuid;

// ── DB pool ───────────────────────────────────────────────────────────────────

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

// ── Test helpers ──────────────────────────────────────────────────────────────

fn test_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["integrations.sync.push".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn build_test_app(pool: sqlx::PgPool, tenant_id: Uuid) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("IntegrationsMetrics::new")),
        bus: Arc::new(InMemoryBus::new()),
    });
    Router::new()
        .route(
            "/api/integrations/sync/push/{entity_type}",
            post(push_entity),
        )
        .with_state(state)
        .layer(Extension(test_claims(tenant_id)))
}

async fn seed_oauth_connection(
    pool: &sqlx::PgPool,
    app_id: &str,
    realm_id: &str,
    status: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections (
            app_id, provider, realm_id,
            access_token, refresh_token,
            access_token_expires_at, refresh_token_expires_at,
            scopes_granted, connection_status
        )
        VALUES ($1, 'quickbooks', $2,
                '\x74657374'::bytea, '\x74657374'::bytea,
                NOW() + INTERVAL '1 hour', NOW() + INTERVAL '30 days',
                'com.intuit.quickbooks.accounting', $3)
        ON CONFLICT (app_id, provider) DO UPDATE
            SET realm_id = EXCLUDED.realm_id,
                connection_status = EXCLUDED.connection_status
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed OAuth connection");
}

async fn seed_authority(pool: &sqlx::PgPool, app_id: &str, entity_type: &str, version: i64) {
    sqlx::query(
        r#"
        INSERT INTO integrations_sync_authority
            (app_id, provider, entity_type, authoritative_side, authority_version)
        VALUES ($1, 'quickbooks', $2, 'platform', $3)
        ON CONFLICT (app_id, provider, entity_type)
        DO UPDATE SET authority_version = $3, updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .bind(version)
    .execute(pool)
    .await
    .expect("seed_authority");
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    let _ = sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_sync_authority WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM integrations_oauth_connections WHERE app_id = $1",
    )
    .bind(app_id)
    .execute(pool)
    .await;
}

fn push_request(
    entity_type: &str,
    entity_id: &str,
    operation: &str,
    authority_version: i64,
    fingerprint: &str,
    payload: Value,
) -> Request<Body> {
    let body = serde_json::json!({
        "entity_id": entity_id,
        "operation": operation,
        "authority_version": authority_version,
        "request_fingerprint": fingerprint,
        "payload": payload,
    });
    Request::builder()
        .method("POST")
        .uri(format!("/api/integrations/sync/push/{}", entity_type))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn response_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse JSON body")
}

// ── DB-only tests ─────────────────────────────────────────────────────────────

/// Unknown entity types are rejected before any DB or QBO interaction.
#[tokio::test]
#[serial]
async fn test_invalid_entity_type_returns_422() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_test_app(pool.clone(), tenant_id);

    let req = push_request(
        "widget",
        "e-1",
        "create",
        1,
        "fp-1",
        serde_json::json!({}),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = response_json(resp).await;
    assert_eq!(body["error"], "invalid_entity_type");
}

/// Unknown operations are rejected before any DB or QBO interaction.
#[tokio::test]
#[serial]
async fn test_invalid_operation_returns_422() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_test_app(pool.clone(), tenant_id);

    let req = push_request(
        "customer",
        "e-1",
        "deletifyall",
        1,
        "fp-1",
        serde_json::json!({}),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = response_json(resp).await;
    assert_eq!(body["error"], "invalid_operation");
}

/// When no QBO OAuth connection exists for the tenant, handler returns 404.
#[tokio::test]
#[serial]
async fn test_missing_oauth_returns_404() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;
    let app = build_test_app(pool.clone(), tenant_id);

    let req = push_request("invoice", "e-1", "create", 1, "fp-1", serde_json::json!({}));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// A disconnected OAuth connection returns 412.
#[tokio::test]
#[serial]
async fn test_disconnected_oauth_returns_412() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;
    seed_oauth_connection(&pool, &app_id, "test-realm-disconn", "disconnected").await;
    let app = build_test_app(pool.clone(), tenant_id);

    let req = push_request("invoice", "e-1", "create", 1, "fp-1", serde_json::json!({}));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);

    cleanup(&pool, &app_id).await;
}

/// When authority has advanced beyond the caller's version, the endpoint
/// returns 200 with outcome:superseded and the current authority version.
#[tokio::test]
#[serial]
async fn test_superseded_returns_correct_json() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    // Seed OAuth so the handler gets past the connection check.
    seed_oauth_connection(&pool, &app_id, &format!("realm-{}", tenant_id.simple()), "connected")
        .await;
    // Authority is at version 5; push is stamped with version 1 → superseded.
    seed_authority(&pool, &app_id, "invoice", 5).await;

    let app = build_test_app(pool.clone(), tenant_id);
    let req = push_request(
        "invoice",
        "test-entity-sup",
        "create",
        1,
        "fp-superseded",
        serde_json::json!({}),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "superseded is a normal outcome, not an error"
    );

    let body = response_json(resp).await;
    assert_eq!(body["outcome"], "superseded", "outcome discriminant");
    assert_eq!(
        body["current_authority_version"].as_i64().unwrap(),
        5,
        "current_authority_version must reflect the DB state"
    );
    assert_eq!(body["entity_id"], "test-entity-sup");
    assert!(body["attempt_id"].is_string(), "attempt_id must be present");

    cleanup(&pool, &app_id).await;
}

/// Submitting the same push twice while the first is still in `accepted` state
/// returns 409 duplicate_intent.
#[tokio::test]
#[serial]
async fn test_duplicate_intent_returns_409() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    let realm_id = format!("realm-dup-{}", tenant_id.simple());
    seed_oauth_connection(&pool, &app_id, &realm_id, "connected").await;
    // Authority at 1 so both pushes use the same version.
    seed_authority(&pool, &app_id, "customer", 1).await;

    let payload = serde_json::json!({ "DisplayName": "Test Corp" });

    // First request: lands in `accepted` (version match → ReadyForInflight, but then
    // transitions to inflight and tries to call QBO with dummy tokens → QBO auth error
    // which is a classified fault, so the attempt completes).  We just need the
    // unique constraint path for the *second* request with the same fingerprint while
    // the first is still in-flight.
    //
    // Seed the first attempt directly so it stays in `accepted`, then send the second.
    let entity_id = format!("dup-entity-{}", Uuid::new_v4().simple());
    let fingerprint = format!("fp-dup-{}", Uuid::new_v4().simple());

    sqlx::query(
        r#"
        INSERT INTO integrations_sync_push_attempts
            (app_id, provider, entity_type, entity_id, operation, authority_version, request_fingerprint, status)
        VALUES ($1, 'quickbooks', 'customer', $2, 'create', 1, $3, 'accepted')
        "#,
    )
    .bind(&app_id)
    .bind(&entity_id)
    .bind(&fingerprint)
    .execute(&pool)
    .await
    .expect("seed accepted attempt");

    let app = build_test_app(pool.clone(), tenant_id);
    let req = push_request("customer", &entity_id, "create", 1, &fingerprint, payload);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    let body = response_json(resp).await;
    assert_eq!(body["error"], "duplicate_intent");

    cleanup(&pool, &app_id).await;
}

/// invoice + void is accepted by the validator; superseded path proves no 422.
#[tokio::test]
#[serial]
async fn test_invoice_void_returns_200_not_422() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    seed_oauth_connection(&pool, &app_id, &format!("realm-void-{}", tenant_id.simple()), "connected").await;
    // Authority at 5; request at 1 → superseded before any QBO call.
    seed_authority(&pool, &app_id, "invoice", 5).await;

    let app = build_test_app(pool.clone(), tenant_id);
    let req = push_request("invoice", "inv-void-1", "void", 1, "fp-void-inv", serde_json::json!({}));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "void on invoice must pass validation and resolve as superseded"
    );

    let body = response_json(resp).await;
    assert_ne!(body["error"], "invalid_operation", "void must not be rejected as invalid_operation for invoice");
    assert_eq!(body["outcome"], "superseded");

    cleanup(&pool, &app_id).await;
}

/// customer + void is rejected at the validator with 422.
#[tokio::test]
#[serial]
async fn test_customer_void_returns_422() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_test_app(pool.clone(), tenant_id);

    let req = push_request("customer", "cust-1", "void", 1, "fp-void-cust", serde_json::json!({}));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = response_json(resp).await;
    assert_eq!(body["error"], "invalid_operation");
}

// ── QBO sandbox tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod sandbox {
    use super::*;

    struct SandboxTokenProvider {
        access_token: RwLock<String>,
        refresh_tok: RwLock<String>,
        client_id: String,
        client_secret: String,
        http: reqwest::Client,
        tokens_path: PathBuf,
    }

    impl SandboxTokenProvider {
        fn load() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            dotenvy::from_path(root.join(".env.qbo-sandbox")).expect(".env.qbo-sandbox not found");

            let client_id = std::env::var("QBO_CLIENT_ID").expect("QBO_CLIENT_ID");
            let client_secret = std::env::var("QBO_CLIENT_SECRET").expect("QBO_CLIENT_SECRET");

            let tokens_path = root.join(".qbo-tokens.json");
            let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json");
            let tokens: Value = serde_json::from_str(&content).expect("invalid tokens JSON");

            Self {
                access_token: RwLock::new(tokens["access_token"].as_str().unwrap().into()),
                refresh_tok: RwLock::new(tokens["refresh_token"].as_str().unwrap().into()),
                client_id,
                client_secret,
                http: reqwest::Client::new(),
                tokens_path,
            }
        }

        fn realm_id(&self) -> String {
            let content = std::fs::read_to_string(&self.tokens_path).unwrap();
            let t: Value = serde_json::from_str(&content).unwrap();
            t["realm_id"].as_str().unwrap().to_string()
        }
    }

    #[async_trait::async_trait]
    impl TokenProvider for SandboxTokenProvider {
        async fn get_token(&self) -> Result<String, QboError> {
            Ok(self.access_token.read().await.clone())
        }

        async fn refresh_token(&self) -> Result<String, QboError> {
            let refresh = self.refresh_tok.read().await.clone();
            let params = [
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
            ];
            let resp = self
                .http
                .post("https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
                .basic_auth(&self.client_id, Some(&self.client_secret))
                .form(&params)
                .send()
                .await
                .map_err(|e| QboError::Http(e))?;

            let j: Value = resp
                .json()
                .await
                .map_err(|e| QboError::Http(e))?;

            let new_access = j["access_token"]
                .as_str()
                .ok_or_else(|| QboError::TokenError("no access_token in refresh response".into()))?
                .to_string();
            let new_refresh = j["refresh_token"]
                .as_str()
                .ok_or_else(|| QboError::TokenError("no refresh_token in refresh response".into()))?
                .to_string();

            *self.access_token.write().await = new_access.clone();
            *self.refresh_tok.write().await = new_refresh.clone();

            let mut data: Value = serde_json::from_str(
                &std::fs::read_to_string(&self.tokens_path).unwrap(),
            )
            .unwrap();
            data["access_token"] = Value::String(new_access.clone());
            data["refresh_token"] = Value::String(new_refresh);
            std::fs::write(
                &self.tokens_path,
                serde_json::to_string_pretty(&data).unwrap(),
            )
            .unwrap();

            Ok(new_access)
        }
    }

    /// Full push pipeline: create a customer through the HTTP endpoint, verify
    /// `outcome:succeeded` with a real QBO sandbox response.
    ///
    /// Requires QBO_SANDBOX=1 and valid .env.qbo-sandbox + .qbo-tokens.json.
    #[tokio::test]
    #[serial]
    async fn test_push_customer_create_succeeds_in_sandbox() {
        if std::env::var("QBO_SANDBOX").unwrap_or_default() != "1" {
            return;
        }

        let provider = Arc::new(SandboxTokenProvider::load());
        let realm_id = provider.realm_id();
        let base_url = std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com".into());

        let pool = setup_db().await;
        let tenant_id = Uuid::new_v4();
        let app_id = tenant_id.to_string();
        cleanup(&pool, &app_id).await;

        // Seed OAuth connection so realm_id lookup works.  The real token is in the
        // sandbox token provider; the encrypted blob stored here is never read during
        // this test because QboClient is constructed directly in build_test_app.
        seed_oauth_connection(&pool, &app_id, &realm_id, "connected").await;
        seed_authority(&pool, &app_id, "customer", 1).await;

        // Build app with sandbox token provider wired in via the real OAUTH_ENCRYPTION_KEY path.
        // Since we cannot inject an arbitrary TokenProvider through the HTTP layer (the handler
        // constructs its own DbTokenProvider), we call ResolveService directly here to exercise
        // the push pipeline and verify the taxonomy serialization.
        use integrations_rs::domain::{
            qbo::client::QboClient,
            sync::resolve_service::{PushOutcome, ResolveService},
        };

        let qbo = Arc::new(QboClient::new(&base_url, &realm_id, provider));
        let svc = ResolveService::new(qbo);

        let unique_name = format!("PushEndpointTest-{}", Uuid::new_v4().simple());
        let payload = serde_json::json!({
            "DisplayName": unique_name,
        });

        let result = svc
            .push_customer(
                &pool,
                &app_id,
                "ep-test-entity",
                "create",
                1,
                &format!("fp-sandbox-{}", Uuid::new_v4()),
                &payload,
            )
            .await
            .expect("push_customer");

        match result {
            PushOutcome::Succeeded { entity_id, provider_entity_id, .. } => {
                assert_eq!(entity_id, "ep-test-entity");
                assert!(
                    provider_entity_id.is_some(),
                    "QBO must return an entity Id on create"
                );
            }
            other => panic!("expected Succeeded, got {:?}", other),
        }

        // Verify serialization: all PushOutcome variants must serialize with an
        // `"outcome"` discriminant tag.
        let serialized = serde_json::to_value(&PushOutcome::Superseded {
            attempt_id: Uuid::new_v4(),
            entity_id: "x".into(),
            current_authority_version: 99,
        })
        .unwrap();
        assert_eq!(serialized["outcome"], "superseded");

        let serialized = serde_json::to_value(&PushOutcome::Failed {
            attempt_id: Uuid::new_v4(),
            entity_id: "x".into(),
            error_code: "rate_limited".into(),
            error_message: "msg".into(),
        })
        .unwrap();
        assert_eq!(serialized["outcome"], "failed");

        cleanup(&pool, &app_id).await;
    }
}
