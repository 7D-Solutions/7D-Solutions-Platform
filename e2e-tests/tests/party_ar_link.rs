//! E2E: Party Master → AR invoice link (bd-4vry)
//!
//! Proves the cross-module integration: create a party in Party Master, create
//! an AR invoice with that party_id, retrieve the invoice via the GET endpoint,
//! and verify the party_id is stored and returned correctly.
//!
//! Also documents current behavior for invalid party_id:
//! - No DB-level FK constraint on ar_invoices.party_id (validation at HTTP layer)
//! - HTTP route returns 422 when party_id is not found in Party Master
//! - HTTP route returns 503 when Party Master service is unreachable
//!
//! ## Pattern
//! - In-process party Axum server on ephemeral TCP port (party-postgres 5448)
//! - In-process AR Axum router (ar-postgres 5434) for both mutations and reads
//! - VerifiedClaims injected into AR mutation requests (bypasses RequirePermissionsLayer)
//! - PARTY_MASTER_URL env var points to ephemeral party server
//! - serial_test::serial prevents env var races across parallel test threads
//!
//! ## Services required
//! - party-postgres at localhost:5448
//! - ar-postgres at localhost:5434
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- party_ar_link --nocapture
//! ```

mod common;

use ar_rs::{metrics::ArMetrics, routes::ar_router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use common::{get_ar_pool, get_party_pool};
use party_rs::{
    domain::party::{service as party_service, CreateCompanyRequest},
    http as party_http,
    metrics::PartyMetrics,
    AppState as PartyAppState,
};
use security::{ActorType, VerifiedClaims};
use serde_json::{json, Value};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

/// app_id must match the hardcoded "test-app" used by AR route handlers.
const AR_APP_ID: &str = "test-app";

// ============================================================================
// Helpers
// ============================================================================

/// Run AR migrations idempotently.
async fn run_ar_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ar/db/migrations")
        .run(pool)
        .await
        .expect("AR migrations failed");
}

/// Run party migrations idempotently.
async fn run_party_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

/// Build an in-process AR router wired to the real AR pool.
fn make_ar_router(pool: PgPool) -> axum::Router {
    let metrics = Arc::new(ArMetrics::new().expect("AR metrics init failed"));
    // AppState is used by health/version routes; ar_router takes PgPool directly
    drop(metrics); // ar_router does not accept AppState
    ar_router(pool)
}

/// Create fake VerifiedClaims with ar.mutate permission so the
/// RequirePermissionsLayer passes in the in-process AR router.
fn make_verified_claims() -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: None,
        roles: vec![],
        perms: vec!["ar.mutate".to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "test".to_string(),
    }
}

/// Send a request to the AR router and return (status, parsed body).
async fn ar_send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    with_auth: bool,
) -> (StatusCode, Value) {
    let body_bytes = match &body {
        Some(v) => v.to_string().into_bytes(),
        None => vec![],
    };

    let mut builder = Request::builder().method(method).uri(uri);

    if !body_bytes.is_empty() {
        builder = builder.header("content-type", "application/json");
    }

    let mut req = builder
        .body(Body::from(body_bytes))
        .expect("request build failed");

    if with_auth {
        req.extensions_mut().insert(make_verified_claims());
    }

    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let resp_bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&resp_bytes).unwrap_or(json!({}));
    (status, parsed)
}

/// Create an AR customer via direct SQL with the hardcoded AR app_id.
async fn create_ar_customer(pool: &PgPool) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, 'Party Link Test Customer', 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(AR_APP_ID)
    .bind(format!("party-link-{}@test.example", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("create AR customer failed")
}

/// Delete AR invoices and customers created by this test run.
async fn cleanup_ar(pool: &PgPool, customer_id: i32) {
    sqlx::query("DELETE FROM events_outbox WHERE app_id = $1")
        .bind(AR_APP_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1 AND ar_customer_id = $2")
        .bind(AR_APP_ID)
        .bind(customer_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE id = $1")
        .bind(customer_id)
        .execute(pool)
        .await
        .ok();
}

/// Delete party rows created by this test run (matched by unique display_name prefix).
async fn cleanup_party(pool: &PgPool, party_id: Uuid) {
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(AR_APP_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_companies WHERE party_id = $1")
        .bind(party_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_parties WHERE id = $1")
        .bind(party_id)
        .execute(pool)
        .await
        .ok();
}

/// Spawn an in-process Party Master HTTP server on an ephemeral port.
/// Returns the port number. Sets PARTY_MASTER_URL env var to point to it.
///
/// # Safety
/// Caller must hold the `serial` lock (use `#[serial]`) to prevent env var races.
async fn spawn_party_server(party_pool: PgPool) -> u16 {
    let metrics = Arc::new(PartyMetrics::new().expect("party metrics init failed"));
    let state = Arc::new(PartyAppState {
        pool: party_pool,
        metrics,
    });
    let router = party_http::router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port for party server");
    let port = listener
        .local_addr()
        .expect("get party server port")
        .port();

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("party server error");
    });

    // Give the server a moment to be ready
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Safety: serial lock prevents concurrent env var mutation; env::set_var is unsafe since Rust 1.83
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var(
            "PARTY_MASTER_URL",
            format!("http://127.0.0.1:{}", port),
        );
    }

    port
}

// ============================================================================
// Test 1: Valid party_id — create, store, retrieve, verify
// ============================================================================

/// Main E2E proof: party created in Party Master → AR invoice with party_id →
/// GET invoice returns party_id → DB confirms storage.
#[tokio::test]
#[serial]
async fn test_party_ar_link_valid_party_id() {
    // ── Connect to databases ──────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ar_pool = get_ar_pool().await;
    run_ar_migrations(&ar_pool).await;

    // ── Start party HTTP server + set PARTY_MASTER_URL ───────────────────
    let _party_port = spawn_party_server(party_pool.clone()).await;
    println!(
        "Party Master server running at {}",
        std::env::var("PARTY_MASTER_URL").unwrap()
    );

    // ── Build in-process AR router ────────────────────────────────────────
    let ar = make_ar_router(ar_pool.clone());

    // ── Step 1: Create a company party in Party Master ───────────────────
    let run_id = Uuid::new_v4();
    let company_req = CreateCompanyRequest {
        display_name: format!("PartyLink Corp {}", &run_id.to_string()[..8]),
        legal_name: format!("PartyLink Corporation {}", &run_id.to_string()[..8]),
        trade_name: None,
        registration_number: None,
        tax_id: None,
        country_of_incorporation: None,
        industry_code: None,
        founded_date: None,
        employee_count: None,
        annual_revenue_cents: None,
        currency: Some("usd".to_string()),
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

    let party_view =
        party_service::create_company(&party_pool, AR_APP_ID, &company_req, run_id.to_string())
            .await
            .expect("create_company failed");

    let party_id = party_view.party.id;
    assert_eq!(party_view.party.app_id, AR_APP_ID);
    assert_eq!(party_view.party.party_type, "company");
    assert_eq!(party_view.party.status, "active");
    println!("Created party: {}", party_id);

    // ── Step 2: Create AR customer ────────────────────────────────────────
    let customer_id = create_ar_customer(&ar_pool).await;
    println!("Created AR customer: {}", customer_id);

    // ── Step 3: POST /api/ar/invoices with valid party_id ─────────────────
    let invoice_body = json!({
        "ar_customer_id": customer_id,
        "amount_cents": 15000,
        "currency": "usd",
        "status": "draft",
        "party_id": party_id.to_string()
    });

    let (create_status, create_body) = ar_send(
        &ar,
        "POST",
        "/api/ar/invoices",
        Some(invoice_body),
        true, // inject VerifiedClaims with ar.mutate
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "POST /api/ar/invoices must return 201; body={}",
        create_body
    );

    let invoice_id = create_body["id"]
        .as_i64()
        .expect("invoice id must be integer in create response");

    let returned_party_id = create_body["party_id"]
        .as_str()
        .expect("party_id must be in create invoice response; body={}");
    assert_eq!(
        returned_party_id,
        party_id.to_string(),
        "create response party_id must match the party we created"
    );
    println!("Created invoice: {} with party_id: {}", invoice_id, returned_party_id);

    // ── Step 4: GET /api/ar/invoices/:id — verify party_id in response ────
    let (get_status, get_body) = ar_send(
        &ar,
        "GET",
        &format!("/api/ar/invoices/{}", invoice_id),
        None,
        false, // GET is a read route — no auth needed
    )
    .await;

    assert_eq!(
        get_status,
        StatusCode::OK,
        "GET /api/ar/invoices/{} must return 200; body={}",
        invoice_id,
        get_body
    );

    let get_party_id = get_body["party_id"]
        .as_str()
        .expect("party_id must be present in GET invoice response; body={}");
    assert_eq!(
        get_party_id,
        party_id.to_string(),
        "GET invoice party_id must match the party we created; body={}",
        get_body
    );
    println!("GET invoice party_id verified: {}", get_party_id);

    // ── Step 5: Verify party_id persisted in DB ───────────────────────────
    let db_party_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT party_id FROM ar_invoices WHERE id = $1 AND app_id = $2",
    )
    .bind(invoice_id as i32)
    .bind(AR_APP_ID)
    .fetch_one(&ar_pool)
    .await
    .expect("DB query for party_id failed");

    assert_eq!(
        db_party_id,
        Some(party_id),
        "party_id must be persisted in ar_invoices table; invoice_id={}",
        invoice_id
    );
    println!("DB party_id confirmed: {:?}", db_party_id);

    // ── Cleanup ────────────────────────────────────────────────────────────
    cleanup_ar(&ar_pool, customer_id).await;
    cleanup_party(&party_pool, party_id).await;
}

// ============================================================================
// Test 2: Invalid party_id — document current behavior
// ============================================================================

/// Documents current behavior when party_id is a random non-existent UUID.
///
/// With Party Master running:
///   → AR service calls Party Master → 404 → AR returns 422 Unprocessable Entity
///
/// With Party Master unreachable:
///   → AR service gets connection error → AR returns 503 Service Unavailable
///
/// Either response is acceptable for this baseline documentation test.
/// The bd-737s bead implements strict validation enforcement.
#[tokio::test]
#[serial]
async fn test_party_ar_link_invalid_party_id_behavior() {
    // ── Connect to databases ──────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ar_pool = get_ar_pool().await;
    run_ar_migrations(&ar_pool).await;

    // ── Start party HTTP server + set PARTY_MASTER_URL ───────────────────
    let _party_port = spawn_party_server(party_pool.clone()).await;

    // ── Build in-process AR router ────────────────────────────────────────
    let ar = make_ar_router(ar_pool.clone());

    // ── Create AR customer ────────────────────────────────────────────────
    let customer_id = create_ar_customer(&ar_pool).await;

    // ── POST invoice with a random non-existent party_id ─────────────────
    let random_party_id = Uuid::new_v4();
    let invoice_body = json!({
        "ar_customer_id": customer_id,
        "amount_cents": 5000,
        "currency": "usd",
        "status": "draft",
        "party_id": random_party_id.to_string()
    });

    let (status, body) = ar_send(
        &ar,
        "POST",
        "/api/ar/invoices",
        Some(invoice_body),
        true,
    )
    .await;

    println!(
        "Invalid party_id behavior: HTTP {} — body={}",
        status, body
    );

    // Document current behavior: 422 (party not found) or 503 (party unreachable).
    // Test PASSES in both cases — this is a baseline documentation test.
    let is_expected_rejection = status == StatusCode::UNPROCESSABLE_ENTITY
        || status == StatusCode::SERVICE_UNAVAILABLE;

    assert!(
        is_expected_rejection,
        "Expected 422 (party not found) or 503 (party unreachable) for unknown party_id; \
         got HTTP {} body={}",
        status, body
    );

    if status == StatusCode::UNPROCESSABLE_ENTITY {
        println!("✅ Validation active: unknown party_id correctly rejected with 422");
    } else if status == StatusCode::SERVICE_UNAVAILABLE {
        println!("ℹ️  Party service unavailable (503) — validation would reject with 422 when live");
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    cleanup_ar(&ar_pool, customer_id).await;
}

// ============================================================================
// Test 3: party_id column accepts NULL (optional field baseline)
// ============================================================================

/// Baseline: AR invoice without party_id stores NULL in party_id column.
/// Ensures party_id remains optional and doesn't break existing invoice flows.
#[tokio::test]
#[serial]
async fn test_party_ar_link_invoice_without_party_id() {
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ar_pool = get_ar_pool().await;
    run_ar_migrations(&ar_pool).await;

    // Start party server (even though we won't use it for this test)
    let _party_port = spawn_party_server(party_pool.clone()).await;

    let ar = make_ar_router(ar_pool.clone());
    let customer_id = create_ar_customer(&ar_pool).await;

    // Create invoice WITHOUT party_id
    let invoice_body = json!({
        "ar_customer_id": customer_id,
        "amount_cents": 2500,
        "currency": "usd",
        "status": "draft"
        // No party_id field
    });

    let (create_status, create_body) = ar_send(
        &ar,
        "POST",
        "/api/ar/invoices",
        Some(invoice_body),
        true,
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "Invoice without party_id must be accepted; body={}",
        create_body
    );

    let invoice_id = create_body["id"]
        .as_i64()
        .expect("invoice id in create response");

    // party_id field should be null in response
    assert!(
        create_body["party_id"].is_null(),
        "party_id must be null when not provided; body={}",
        create_body
    );

    // Verify via GET
    let (get_status, get_body) = ar_send(
        &ar,
        "GET",
        &format!("/api/ar/invoices/{}", invoice_id),
        None,
        false,
    )
    .await;

    assert_eq!(get_status, StatusCode::OK, "GET must return 200; body={}", get_body);
    assert!(
        get_body["party_id"].is_null(),
        "GET invoice party_id must be null when not set; body={}",
        get_body
    );

    // Verify in DB
    let db_party_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT party_id FROM ar_invoices WHERE id = $1 AND app_id = $2",
    )
    .bind(invoice_id as i32)
    .bind(AR_APP_ID)
    .fetch_one(&ar_pool)
    .await
    .expect("DB query failed");

    assert!(
        db_party_id.is_none(),
        "party_id must be NULL in DB when not provided; got {:?}",
        db_party_id
    );

    println!("✅ Invoice without party_id: party_id is NULL throughout (create, GET, DB)");

    // Cleanup
    cleanup_ar(&ar_pool, customer_id).await;
}
