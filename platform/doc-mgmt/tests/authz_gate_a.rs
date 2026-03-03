use axum::{body::Body, http::Request};
use chrono::{Duration, Utc};
use doc_mgmt::{handlers::AppState, routes::api_router};
use security::{ActorType, VerifiedClaims};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

fn build_claims(perms: &[&str]) -> VerifiedClaims {
    let now = Utc::now();
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: None,
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|p| (*p).to_string()).collect(),
        actor_type: ActorType::User,
        issued_at: now,
        expires_at: now + Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "v1".to_string(),
    }
}

fn test_app() -> axum::Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgresql://doc_mgmt_user:doc_mgmt_pass@127.0.0.1:59999/doc_mgmt_db")
        .expect("lazy pool");
    api_router(std::sync::Arc::new(AppState { db: pool }))
}

#[tokio::test]
async fn create_document_requires_mutate_permission() {
    let app = test_app();
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/documents")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"doc_number":"AUTHZ-1","title":"Doc","doc_type":"spec","body":{}}"#,
        ))
        .expect("request");
    req.extensions_mut()
        .insert(build_claims(&["doc_mgmt.read"]));

    let resp = app.oneshot(req).await.expect("router response");
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn release_requires_mutate_permission() {
    let app = test_app();
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/api/documents/{}/release", Uuid::new_v4()))
        .body(Body::empty())
        .expect("request");
    req.extensions_mut()
        .insert(build_claims(&["doc_mgmt.read"]));

    let resp = app.oneshot(req).await.expect("router response");
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn distribute_requires_mutate_permission() {
    let app = test_app();
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/api/documents/{}/distributions", Uuid::new_v4()))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"recipient_ref":"qa@fireproof.test","channel":"email","template_key":"dist"}"#,
        ))
        .expect("request");
    req.extensions_mut()
        .insert(build_claims(&["doc_mgmt.read"]));

    let resp = app.oneshot(req).await.expect("router response");
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_mutation_endpoint_requires_mutate_permission() {
    let app = test_app();
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/retention-policies")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"doc_type":"spec","retention_days":365}"#))
        .expect("request");
    req.extensions_mut()
        .insert(build_claims(&["doc_mgmt.read"]));

    let resp = app.oneshot(req).await.expect("router response");
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn read_endpoint_requires_read_permission() {
    let app = test_app();
    let mut req = Request::builder()
        .method("GET")
        .uri("/api/documents")
        .body(Body::empty())
        .expect("request");
    req.extensions_mut()
        .insert(build_claims(&["doc_mgmt.mutate"]));

    let resp = app.oneshot(req).await.expect("router response");
    assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
}
