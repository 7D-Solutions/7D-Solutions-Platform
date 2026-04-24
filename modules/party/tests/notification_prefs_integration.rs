//! Integration tests for notification preference PATCH endpoints (bd-kv15d).
//!
//! All tests hit real Postgres on port 5448. No mocks.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p party-rs -- notification_prefs

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Extension, Router,
};
use chrono::Utc;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use party_rs::domain::contact_service;
use party_rs::domain::contact::CreateContactRequest;
use party_rs::domain::party::service::create_company;
use party_rs::domain::party::CreateCompanyRequest;
use party_rs::{http, metrics, AppState};
use security::{claims::ActorType, VerifiedClaims};

// ============================================================================
// Helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://party_user:party_pass@localhost:5448/party_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to party test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("party migrations");
    pool
}

fn admin_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["tenant_admin".to_string()],
        perms: vec!["party.mutate".to_string(), "party.read".to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn unprivileged_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["viewer".to_string()],
        perms: vec!["party.read".to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn build_router_with_claims(pool: sqlx::PgPool, claims: VerifiedClaims) -> axum::Router {
    let party_metrics = Arc::new(metrics::PartyMetrics::new().expect("metrics"));
    let app_state = Arc::new(AppState {
        pool,
        metrics: party_metrics,
    });
    http::router(app_state).layer(Extension(claims))
}

fn build_router_no_auth(pool: sqlx::PgPool) -> Router {
    let party_metrics = Arc::new(metrics::PartyMetrics::new().expect("metrics"));
    let app_state = Arc::new(AppState {
        pool,
        metrics: party_metrics,
    });
    let maybe_verifier: Option<Arc<security::JwtVerifier>> = None;
    http::router(app_state).layer(axum::middleware::from_fn_with_state(
        maybe_verifier,
        security::optional_claims_mw,
    ))
}

async fn create_test_party(pool: &sqlx::PgPool, tenant_id: Uuid) -> Uuid {
    let app_id = tenant_id.to_string();
    let req = CreateCompanyRequest {
        display_name: format!("Test Co {}", Uuid::new_v4().simple()),
        legal_name: "Test Co LLC".to_string(),
        trade_name: None,
        registration_number: None,
        tax_id: None,
        country_of_incorporation: None,
        industry_code: None,
        founded_date: None,
        employee_count: None,
        annual_revenue_cents: None,
        currency: None,
        email: None,
        phone: None,
        website: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: None,
        metadata: None,
    };
    create_company(pool, &app_id, &req, Uuid::new_v4().to_string())
        .await
        .expect("create test party")
        .party
        .id
}

async fn create_test_contact(pool: &sqlx::PgPool, party_id: Uuid, tenant_id: Uuid) -> Uuid {
    let app_id = tenant_id.to_string();
    let req = CreateContactRequest {
        first_name: "Ship".to_string(),
        last_name: Some("To".to_string()),
        email: Some(format!("shipto-{}@test.com", Uuid::new_v4().simple())),
        phone: None,
        role: None,
        is_primary: Some(false),
        metadata: None,
    };
    contact_service::create_contact(pool, &app_id, party_id, &req, Uuid::new_v4().to_string())
        .await
        .expect("create test contact")
        .id
}

// ============================================================================
// Tests
// ============================================================================

/// PATCH with valid data updates the party row.
#[tokio::test]
#[serial]
async fn notification_prefs_patch_updates_party_row() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;

    let router = build_router_with_claims(pool.clone(), admin_claims(tenant_id));
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "notification_events": ["shipped", "delivered"],
                "notification_channels": ["email"]
            })
            .to_string(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify persisted
    let row: (serde_json::Value, serde_json::Value) = sqlx::query_as(
        "SELECT notification_events, notification_channels FROM party_parties WHERE id = $1",
    )
    .bind(party_id)
    .fetch_one(&pool)
    .await
    .expect("fetch party");
    let events: Vec<String> = serde_json::from_value(row.0).unwrap();
    let channels: Vec<String> = serde_json::from_value(row.1).unwrap();
    assert!(events.contains(&"shipped".to_string()));
    assert!(events.contains(&"delivered".to_string()));
    assert!(channels.contains(&"email".to_string()));
}

/// Unknown event value returns 400.
#[tokio::test]
#[serial]
async fn notification_prefs_rejects_unknown_event() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;

    let router = build_router_with_claims(pool, admin_claims(tenant_id));
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "notification_events": ["teleported"] }).to_string(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Unknown channel value returns 400.
#[tokio::test]
#[serial]
async fn notification_prefs_rejects_unknown_channel() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;

    let router = build_router_with_claims(pool, admin_claims(tenant_id));
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "notification_channels": ["carrier_pigeon"] }).to_string(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Ship-to contact override wins when non-null.
#[tokio::test]
#[serial]
async fn notification_prefs_ship_to_override_wins() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;
    let contact_id = create_test_contact(&pool, party_id, tenant_id).await;

    // Set party-level prefs
    sqlx::query(
        "UPDATE party_parties SET notification_events = $1, notification_channels = $2 WHERE id = $3",
    )
    .bind(serde_json::json!(["shipped"]))
    .bind(serde_json::json!(["email"]))
    .bind(party_id)
    .execute(&pool)
    .await
    .unwrap();

    // Set contact override for channels only
    let router = build_router_with_claims(pool.clone(), admin_claims(tenant_id));
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/ship-to/{contact_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "notification_channels": ["sms"] }).to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Resolve: events = party (shipped), channels = contact (sms)
    let (party_events, party_channels): (serde_json::Value, serde_json::Value) =
        sqlx::query_as("SELECT notification_events, notification_channels FROM party_parties WHERE id = $1")
            .bind(party_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (contact_events, contact_channels): (Option<serde_json::Value>, Option<serde_json::Value>) =
        sqlx::query_as("SELECT notification_events, notification_channels FROM party_contacts WHERE id = $1")
            .bind(contact_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let (eff_events, eff_channels) = party_rs::domain::notifications::resolve_for_contact(
        serde_json::from_value::<Vec<String>>(party_events).unwrap(),
        serde_json::from_value::<Vec<String>>(party_channels).unwrap(),
        contact_events.and_then(|v| serde_json::from_value(v).ok()),
        contact_channels.and_then(|v| serde_json::from_value(v).ok()),
    );
    assert_eq!(eff_events, vec!["shipped"]);
    assert_eq!(eff_channels, vec!["sms"]);
}

/// Ship-to NULL field inherits from party.
#[tokio::test]
#[serial]
async fn notification_prefs_ship_to_null_inherits() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;
    let contact_id = create_test_contact(&pool, party_id, tenant_id).await;

    sqlx::query(
        "UPDATE party_parties SET notification_channels = $1 WHERE id = $2",
    )
    .bind(serde_json::json!(["email"]))
    .bind(party_id)
    .execute(&pool)
    .await
    .unwrap();

    // Contact has no override (NULL columns) — inherits party values
    let (contact_channels,): (Option<serde_json::Value>,) =
        sqlx::query_as("SELECT notification_channels FROM party_contacts WHERE id = $1")
            .bind(contact_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(contact_channels.is_none(), "contact_channels must start NULL");

    // Resolve with both None → party values
    let (_, eff_channels) = party_rs::domain::notifications::resolve_for_contact(
        vec!["shipped".to_string()],
        vec!["email".to_string()],
        None,
        None,
    );
    assert_eq!(eff_channels, vec!["email"]);
}

/// Clearing a ship-to override (PATCH with null) restores inheritance.
#[tokio::test]
#[serial]
async fn notification_prefs_clearing_ship_to_restores_inherit() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;
    let contact_id = create_test_contact(&pool, party_id, tenant_id).await;

    // Set override
    sqlx::query(
        "UPDATE party_contacts SET notification_channels = $1 WHERE id = $2",
    )
    .bind(serde_json::json!(["sms"]))
    .bind(contact_id)
    .execute(&pool)
    .await
    .unwrap();

    // Clear via PATCH with null
    let router = build_router_with_claims(pool.clone(), admin_claims(tenant_id));
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/ship-to/{contact_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "notification_channels": null }).to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify column is now NULL
    let (channels,): (Option<serde_json::Value>,) =
        sqlx::query_as("SELECT notification_channels FROM party_contacts WHERE id = $1")
            .bind(contact_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(channels.is_none(), "clearing override must set column to NULL");
}

/// Request without tenant_admin or customer_manager role returns 403.
#[tokio::test]
#[serial]
async fn notification_prefs_requires_permission() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;

    // Inject read-only claims (no tenant_admin, no customer_manager)
    let router = build_router_with_claims(pool, unprivileged_claims(tenant_id));
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "notification_events": ["shipped"] }).to_string(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// Non-array body for notification_events is rejected with 400 (JSONB array enforcement).
#[tokio::test]
#[serial]
async fn notification_prefs_enforces_jsonb_array_shape() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let party_id = create_test_party(&pool, tenant_id).await;

    let router = build_router_with_claims(pool, admin_claims(tenant_id));
    // Passing a string instead of an array — should fail validation
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/customers/{party_id}/notifications"))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"notification_events":"shipped"}"#.to_string(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    // Body parser rejects non-array as 422 (serde error); either 400 or 422 is acceptable
    assert!(
        resp.status() == StatusCode::BAD_REQUEST
            || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "non-array field must be rejected, got {}",
        resp.status()
    );
}

/// GIN indexes on party_parties must be usable (seqscan disabled).
#[tokio::test]
#[serial]
async fn notification_prefs_gin_index_party() {
    let pool = setup_db().await;

    // Disable seqscan to force planner to use index if it exists
    sqlx::query("SET enable_seqscan = off")
        .execute(&pool)
        .await
        .expect("set enable_seqscan");

    let plan: (String,) = sqlx::query_as(
        "EXPLAIN SELECT id FROM party_parties WHERE notification_events @> '[\"shipped\"]'::jsonb",
    )
    .fetch_one(&pool)
    .await
    .expect("EXPLAIN");

    sqlx::query("RESET enable_seqscan")
        .execute(&pool)
        .await
        .ok();

    assert!(
        plan.0.contains("GIN") || plan.0.contains("Bitmap") || plan.0.contains("Index"),
        "planner must use GIN index on notification_events with seqscan disabled, got: {}",
        plan.0
    );
}
