use axum::{body::Body, http::Request};
use customer_portal::{
    auth::PortalJwt, build_router, hash_password, metrics::PortalMetrics, AppState,
};
use rand::thread_rng;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5432/customer_portal_db".to_string()
    })
}

fn make_test_keys() -> (String, String) {
    let mut rng = thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
    let public_key = private_key.to_public_key();

    let private_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("private pem")
        .to_string();
    let public_pem = public_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public pem");

    (private_pem, public_pem)
}

async fn test_app() -> Option<(axum::Router, sqlx::PgPool, Arc<PortalJwt>)> {
    let pool = match PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(3))
        .max_connections(5)
        .connect(&test_db_url())
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("skipping e2e test: database unavailable ({err})");
            return None;
        }
    };

    if let Err(err) = sqlx::migrate!("./db/migrations").run(&pool).await {
        eprintln!("skipping e2e test: migration failed ({err})");
        return None;
    }

    sqlx::query("TRUNCATE portal_refresh_tokens, portal_idempotency, events_outbox, portal_users")
        .execute(&pool)
        .await
        .expect("truncate tables");

    let (priv_pem, pub_pem) = make_test_keys();
    let portal_jwt = Arc::new(PortalJwt::new(&priv_pem, &pub_pem).expect("portal jwt"));

    let state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: PortalMetrics::new().expect("metrics"),
        portal_jwt: portal_jwt.clone(),
        config: customer_portal::config::Config {
            database_url: test_db_url(),
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

#[tokio::test]
#[serial]
async fn portal_user_cannot_cross_party_boundary() {
    let Some((app, pool, portal_jwt)) = test_app().await else {
        return;
    };

    let tenant_id = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_a)
    .bind("user@example.com")
    .bind(hash_password("Passw0rd!Passw0rd").expect("hash"))
    .bind("Portal User")
    .execute(&pool)
    .await
    .expect("insert user");

    let token = portal_jwt
        .issue_access_token(
            user_id,
            tenant_id,
            party_a,
            vec![platform_contracts::portal_identity::scopes::DOCUMENTS_READ.to_string()],
            15,
        )
        .expect("token");

    let req = Request::builder()
        .uri(format!("/portal/party/{party_b}/probe"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");

    let res = app.clone().oneshot(req).await.expect("response");
    assert_eq!(res.status(), axum::http::StatusCode::NOT_FOUND);

    let req_ok = Request::builder()
        .uri(format!("/portal/party/{party_a}/probe"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");

    let res_ok = app.oneshot(req_ok).await.expect("response");
    assert_eq!(res_ok.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
#[serial]
async fn auth_failures_are_consistent() {
    let Some((app, pool, _portal_jwt)) = test_app().await else {
        return;
    };

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name, is_active) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind("auth@example.com")
    .bind(hash_password("CorrectPassw0rd!").expect("hash"))
    .bind("Auth User")
    .bind(true)
    .execute(&pool)
    .await
    .expect("insert user");

    let bad_login = serde_json::json!({
        "tenant_id": tenant_id,
        "email": "auth@example.com",
        "password": "wrong-password"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/portal/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(bad_login.to_string()))
        .expect("request");

    let res = app.clone().oneshot(req).await.expect("response");
    assert_eq!(res.status(), axum::http::StatusCode::UNAUTHORIZED);

    sqlx::query("UPDATE portal_users SET is_active = false WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("deactivate");

    let disabled_login = serde_json::json!({
        "tenant_id": tenant_id,
        "email": "auth@example.com",
        "password": "CorrectPassw0rd!"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/portal/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(disabled_login.to_string()))
        .expect("request");

    let res = app.oneshot(req).await.expect("response");
    assert_eq!(res.status(), axum::http::StatusCode::FORBIDDEN);
}

async fn start_doc_mgmt_mock(
    response_body: serde_json::Value,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::{routing::get, Json, Router};
    let app = Router::new().route(
        "/api/documents/{id}/distributions",
        get(move || {
            let payload = response_body.clone();
            async move { Json(payload) }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });

    (format!("http://{}", addr), handle)
}

#[tokio::test]
#[serial]
async fn docs_visibility_requires_distribution_recipient_match() {
    let Some((_app, pool, portal_jwt)) = test_app().await else {
        return;
    };

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind("visible@example.com")
    .bind(hash_password("Passw0rd!Passw0rd").expect("hash"))
    .bind("Portal User")
    .execute(&pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO portal_document_links (id, tenant_id, party_id, document_id, display_title) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(party_id)
    .bind(doc_id)
    .bind("Document A")
    .execute(&pool)
    .await
    .expect("insert link");

    let token = portal_jwt
        .issue_access_token(
            user_id,
            tenant_id,
            party_id,
            vec![platform_contracts::portal_identity::scopes::DOCUMENTS_READ.to_string()],
            15,
        )
        .expect("token");

    let (mock_base_url, handle) = start_doc_mgmt_mock(serde_json::json!({
        "distributions": [
            {"id": Uuid::new_v4(), "recipient_ref": "other@example.com", "status": "delivered"}
        ]
    }))
    .await;

    // Build a fresh app with mock URL
    let state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: PortalMetrics::new().expect("metrics"),
        portal_jwt: portal_jwt.clone(),
        config: customer_portal::config::Config {
            database_url: test_db_url(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cors_origins: vec!["*".to_string()],
            portal_jwt_private_key: "".to_string(),
            portal_jwt_public_key: "".to_string(),
            access_token_ttl_minutes: 15,
            refresh_token_ttl_days: 7,
            doc_mgmt_base_url: mock_base_url.clone(),
            doc_mgmt_bearer_token: None,
        },
    });
    let app = build_router(state);

    let req = Request::builder()
        .uri("/portal/docs")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");

    let res = app.clone().oneshot(req).await.expect("response");
    assert_eq!(res.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        json.get("documents")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or_default(),
        0
    );

    handle.abort();
}

#[tokio::test]
#[serial]
async fn acknowledgments_are_idempotent_and_emit_outbox_event() {
    let Some((_app, pool, portal_jwt)) = test_app().await else {
        return;
    };

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let document_id = Uuid::new_v4();
    let idem = "ack-idem-1";

    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind("ack@example.com")
    .bind(hash_password("Passw0rd!Passw0rd").expect("hash"))
    .bind("Portal User")
    .execute(&pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO portal_document_links (id, tenant_id, party_id, document_id, display_title) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(party_id)
    .bind(document_id)
    .bind("Document B")
    .execute(&pool)
    .await
    .expect("insert link");

    let token = portal_jwt
        .issue_access_token(
            user_id,
            tenant_id,
            party_id,
            vec![
                platform_contracts::portal_identity::scopes::DOCUMENTS_READ.to_string(),
                platform_contracts::portal_identity::scopes::ACKNOWLEDGMENTS_WRITE.to_string(),
            ],
            15,
        )
        .expect("token");

    let state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: PortalMetrics::new().expect("metrics"),
        portal_jwt: portal_jwt.clone(),
        config: customer_portal::config::Config {
            database_url: test_db_url(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cors_origins: vec!["*".to_string()],
            portal_jwt_private_key: "".to_string(),
            portal_jwt_public_key: "".to_string(),
            access_token_ttl_minutes: 15,
            refresh_token_ttl_days: 7,
            doc_mgmt_base_url: "http://127.0.0.1:1".to_string(),
            doc_mgmt_bearer_token: None,
        },
    });
    let app = build_router(state);

    let payload = serde_json::json!({
        "document_id": document_id,
        "ack_type": "received",
        "notes": "ok",
        "idempotency_key": idem
    });

    let req = Request::builder()
        .method("POST")
        .uri("/portal/acknowledgments")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_string()))
        .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    assert_eq!(res.status(), axum::http::StatusCode::OK);

    let req = Request::builder()
        .method("POST")
        .uri("/portal/acknowledgments")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_string()))
        .expect("request");
    let res = app.oneshot(req).await.expect("response");
    assert_eq!(res.status(), axum::http::StatusCode::OK);

    let ack_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM portal_acknowledgments WHERE tenant_id = $1 AND party_id = $2 AND idempotency_key = $3",
    )
    .bind(tenant_id)
    .bind(party_id)
    .bind(idem)
    .fetch_one(&pool)
    .await
    .expect("ack count");
    assert_eq!(ack_count, 1);

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'portal.acknowledgment.recorded'",
    )
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1);
}
