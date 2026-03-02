//! E2E: AP vendor → Party Master link (bd-1pjl)
//!
//! Proves the AP vendor entity links correctly to Party Master via party_id:
//! create a party in Party Master, create an AP vendor with that party_id, verify
//! the vendor record stores it correctly. Also documents behavior with an invalid
//! party_id (feeds into any future AP party validation bead).
//!
//! ## Pattern
//! - In-process party Axum server on ephemeral TCP port (party-postgres 5448)
//! - In-process AP Axum router (ap-postgres 5443) for HTTP mutations and reads
//! - VerifiedClaims injected into AP mutation requests (bypasses RequirePermissionsLayer)
//! - serial_test::serial prevents env var races across parallel test threads
//!
//! ## Services required
//! - ap-postgres at localhost:5443
//! - party-postgres at localhost:5448
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- ap_vendor_party_link_e2e --nocapture
//! ```

mod common;

use ap::{http, metrics::ApMetrics, AppState};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post, put},
    Router,
};
use chrono::Utc;
use common::{get_ap_pool, get_party_pool};
use party_rs::{
    domain::party::{service as party_service, CreateCompanyRequest},
    http as party_http,
    metrics::PartyMetrics,
    AppState as PartyAppState,
};
use security::{permissions, ActorType, RequirePermissionsLayer, VerifiedClaims};
use serde_json::{json, Value};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

const AP_TENANT_ID: &str = "ap-party-link-test-tenant";

// ============================================================================
// Helpers
// ============================================================================

/// Run AP migrations idempotently.
async fn run_ap_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ap/db/migrations")
        .run(pool)
        .await
        .expect("AP migrations failed");
}

/// Run party migrations idempotently.
async fn run_party_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

/// Build an in-process AP router wired to the real AP pool.
///
/// Builds both the read routes (no auth) and write routes (RequirePermissionsLayer).
fn make_ap_router(pool: PgPool) -> axum::Router {
    let metrics = Arc::new(ApMetrics::new().expect("AP metrics init failed"));
    let state = Arc::new(AppState { pool, metrics });

    let ap_mutations = Router::new()
        .route("/api/ap/vendors", post(http::vendors::create_vendor))
        .route(
            "/api/ap/vendors/{vendor_id}",
            put(http::vendors::update_vendor),
        )
        .route(
            "/api/ap/vendors/{vendor_id}/deactivate",
            post(http::vendors::deactivate_vendor),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::AP_MUTATE]))
        .with_state(state.clone());

    Router::new()
        .route("/api/ap/vendors", get(http::vendors::list_vendors))
        .route(
            "/api/ap/vendors/{vendor_id}",
            get(http::vendors::get_vendor),
        )
        .with_state(state)
        .merge(ap_mutations)
}

/// Create fake VerifiedClaims with ap.mutate permission.
fn make_verified_claims() -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: None,
        roles: vec![],
        perms: vec![permissions::AP_MUTATE.to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "test".to_string(),
    }
}

/// Send a request to the AP router and return (status, parsed body).
async fn ap_send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    with_auth: bool,
    tenant_id: &str,
) -> (StatusCode, Value) {
    let body_bytes = match &body {
        Some(v) => v.to_string().into_bytes(),
        None => vec![],
    };

    let mut builder = Request::builder().method(method).uri(uri);
    builder = builder.header("x-tenant-id", tenant_id);

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

/// Spawn an in-process Party Master HTTP server on an ephemeral port.
/// Sets PARTY_MASTER_URL env var to point to it.
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
    let port = listener.local_addr().expect("get party server port").port();

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("party server error");
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Safety: serial lock prevents concurrent env var mutation; env::set_var is unsafe since Rust 1.83
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("PARTY_MASTER_URL", format!("http://127.0.0.1:{}", port));
    }

    port
}

/// Cleanup AP vendors created by this test run.
async fn cleanup_ap_vendors(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' AND aggregate_id IN \
         (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
    )
    .bind(AP_TENANT_ID)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(AP_TENANT_ID)
        .execute(pool)
        .await
        .ok();
}

/// Cleanup party rows created by this test run.
async fn cleanup_party(pool: &PgPool, party_id: Uuid) {
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(AP_TENANT_ID)
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

// ============================================================================
// Test 1: Valid party_id — create, store, retrieve, update
// ============================================================================

/// Main E2E proof: party created in Party Master → AP vendor with party_id →
/// GET vendor returns party_id → PUT updates party_id → DB confirms.
#[tokio::test]
#[serial]
async fn test_ap_vendor_party_link_valid_party_id() {
    // ── Connect to databases ──────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = get_ap_pool().await;
    run_ap_migrations(&ap_pool).await;
    cleanup_ap_vendors(&ap_pool).await;

    // ── Start party HTTP server ──────────────────────────────────────────
    let _party_port = spawn_party_server(party_pool.clone()).await;
    println!(
        "Party Master server running at {}",
        std::env::var("PARTY_MASTER_URL").unwrap()
    );

    // ── Build in-process AP router ────────────────────────────────────────
    let ap = make_ap_router(ap_pool.clone());

    // ── Step 1: Create a company party in Party Master ───────────────────
    let run_id = Uuid::new_v4();
    let company_req = CreateCompanyRequest {
        display_name: format!("AP Vendor Corp {}", &run_id.to_string()[..8]),
        legal_name: format!("AP Vendor Corporation {}", &run_id.to_string()[..8]),
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
        party_service::create_company(&party_pool, AP_TENANT_ID, &company_req, run_id.to_string())
            .await
            .expect("create_company failed");

    let party_id = party_view.party.id;
    assert_eq!(party_view.party.party_type, "company");
    assert_eq!(party_view.party.status, "active");
    println!("Created party: {}", party_id);

    // ── Step 2: POST /api/ap/vendors with valid party_id ─────────────────
    let vendor_name = format!("Test Vendor {}", &run_id.to_string()[..8]);
    let create_body = json!({
        "name": vendor_name,
        "currency": "USD",
        "payment_terms_days": 30,
        "payment_method": "ach",
        "party_id": party_id.to_string()
    });

    let (create_status, create_resp) = ap_send(
        &ap,
        "POST",
        "/api/ap/vendors",
        Some(create_body),
        true,
        AP_TENANT_ID,
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "POST /api/ap/vendors must return 201; body={}",
        create_resp
    );

    let vendor_id = create_resp["vendor_id"]
        .as_str()
        .expect("vendor_id must be in create response")
        .to_string();

    let returned_party_id = create_resp["party_id"]
        .as_str()
        .expect("party_id must be in create response");
    assert_eq!(
        returned_party_id,
        party_id.to_string(),
        "create response party_id must match the party we created"
    );
    println!(
        "Created vendor: {} with party_id: {}",
        vendor_id, returned_party_id
    );

    // ── Step 3: GET /api/ap/vendors/:id — verify party_id in response ─────
    let (get_status, get_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        AP_TENANT_ID,
    )
    .await;

    assert_eq!(
        get_status,
        StatusCode::OK,
        "GET /api/ap/vendors/{} must return 200; body={}",
        vendor_id,
        get_resp
    );

    let get_party_id = get_resp["party_id"]
        .as_str()
        .expect("party_id must be present in GET vendor response");
    assert_eq!(
        get_party_id,
        party_id.to_string(),
        "GET vendor party_id must match the party we created"
    );
    println!("GET vendor party_id verified: {}", get_party_id);

    // ── Step 4: Verify party_id persisted in DB ───────────────────────────
    let vendor_uuid = Uuid::parse_str(&vendor_id).expect("vendor_id must be a UUID");
    let db_party_id: Option<Uuid> =
        sqlx::query_scalar("SELECT party_id FROM vendors WHERE vendor_id = $1 AND tenant_id = $2")
            .bind(vendor_uuid)
            .bind(AP_TENANT_ID)
            .fetch_one(&ap_pool)
            .await
            .expect("DB query for party_id failed");

    assert_eq!(
        db_party_id,
        Some(party_id),
        "party_id must be persisted in vendors table; vendor_id={}",
        vendor_id
    );
    println!("DB party_id confirmed: {:?}", db_party_id);

    // ── Step 5: Create a second party and update vendor party_id ──────────
    let party2_req = CreateCompanyRequest {
        display_name: format!("AP Vendor Corp2 {}", &run_id.to_string()[..8]),
        legal_name: format!("AP Vendor Corporation2 {}", &run_id.to_string()[..8]),
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

    let party2_view = party_service::create_company(
        &party_pool,
        AP_TENANT_ID,
        &party2_req,
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create second party failed");
    let party2_id = party2_view.party.id;

    let update_body = json!({
        "party_id": party2_id.to_string()
    });

    let (put_status, put_resp) = ap_send(
        &ap,
        "PUT",
        &format!("/api/ap/vendors/{}", vendor_id),
        Some(update_body),
        true,
        AP_TENANT_ID,
    )
    .await;

    assert_eq!(
        put_status,
        StatusCode::OK,
        "PUT /api/ap/vendors/{} must return 200; body={}",
        vendor_id,
        put_resp
    );

    let updated_party_id = put_resp["party_id"]
        .as_str()
        .expect("party_id must be in PUT response");
    assert_eq!(
        updated_party_id,
        party2_id.to_string(),
        "PUT response party_id must reflect updated party"
    );
    println!("Updated vendor party_id to: {}", updated_party_id);

    // Verify GET reflects the update
    let (get2_status, get2_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        AP_TENANT_ID,
    )
    .await;

    assert_eq!(
        get2_status,
        StatusCode::OK,
        "GET after update must return 200"
    );
    let get2_party_id = get2_resp["party_id"]
        .as_str()
        .expect("party_id must be in GET response after update");
    assert_eq!(
        get2_party_id,
        party2_id.to_string(),
        "GET after update must reflect new party_id"
    );
    println!("GET after update confirmed: party_id={}", get2_party_id);

    // ── Step 6: Query DB to verify party-based lookup ────────────────────
    let linked_vendors: Vec<Uuid> =
        sqlx::query_scalar("SELECT vendor_id FROM vendors WHERE tenant_id = $1 AND party_id = $2")
            .bind(AP_TENANT_ID)
            .bind(party2_id)
            .fetch_all(&ap_pool)
            .await
            .expect("party-based vendor lookup failed");

    assert!(
        linked_vendors.contains(&vendor_uuid),
        "Vendor must be findable by party_id in DB; linked_vendors={:?}",
        linked_vendors
    );
    println!(
        "Party-based DB lookup verified: {} vendor(s) linked to party {}",
        linked_vendors.len(),
        party2_id
    );

    // ── Cleanup ────────────────────────────────────────────────────────────
    cleanup_ap_vendors(&ap_pool).await;
    cleanup_party(&party_pool, party_id).await;
    cleanup_party(&party_pool, party2_id).await;
}

// ============================================================================
// Test 2: Invalid party_id — document current behavior
// ============================================================================

/// Documents current behavior when party_id is a random non-existent UUID.
///
/// Unlike AR (which validates party_id against Party Master), AP vendors
/// do not currently validate party_id — the field is stored as-is.
/// Expected: 201 Created (no validation at the AP vendor layer).
///
/// This test documents the baseline behavior for any future AP party
/// validation bead.
#[tokio::test]
#[serial]
async fn test_ap_vendor_party_link_invalid_party_id_behavior() {
    // ── Connect to databases ──────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = get_ap_pool().await;
    run_ap_migrations(&ap_pool).await;
    cleanup_ap_vendors(&ap_pool).await;

    // ── Start party HTTP server ──────────────────────────────────────────
    let _party_port = spawn_party_server(party_pool.clone()).await;

    // ── Build in-process AP router ────────────────────────────────────────
    let ap = make_ap_router(ap_pool.clone());

    // ── POST vendor with a random non-existent party_id ──────────────────
    let random_party_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let create_body = json!({
        "name": format!("Invalid Party Vendor {}", &run_id.to_string()[..8]),
        "currency": "USD",
        "payment_terms_days": 30,
        "party_id": random_party_id.to_string()
    });

    let (status, body) = ap_send(
        &ap,
        "POST",
        "/api/ap/vendors",
        Some(create_body),
        true,
        AP_TENANT_ID,
    )
    .await;

    println!("Invalid party_id behavior: HTTP {} — body={}", status, body);

    // AP vendors do not validate party_id — the UUID is stored as-is.
    // Current behavior: 201 Created (no Party Master lookup occurs).
    // A future validation bead may add enforcement (422 on unknown party_id).
    assert_eq!(
        status,
        StatusCode::CREATED,
        "AP vendors accept any party_id UUID without validation (baseline behavior); \
         got HTTP {} body={}",
        status,
        body
    );

    let returned_party_id = body["party_id"]
        .as_str()
        .expect("party_id must be in response");
    assert_eq!(
        returned_party_id,
        random_party_id.to_string(),
        "Unvalidated party_id must be stored and returned as-is"
    );

    println!(
        "✅ Baseline documented: AP vendor accepts invalid party_id with 201 \
         (no Party Master validation at vendor layer)"
    );

    // ── Cleanup ────────────────────────────────────────────────────────────
    cleanup_ap_vendors(&ap_pool).await;
}

// ============================================================================
// Test 3: Vendor without party_id (party_id is optional)
// ============================================================================

/// Baseline: AP vendor without party_id stores NULL.
/// Ensures party_id remains optional and doesn't break existing vendor flows.
#[tokio::test]
#[serial]
async fn test_ap_vendor_without_party_id() {
    // ── Connect to databases ──────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = get_ap_pool().await;
    run_ap_migrations(&ap_pool).await;
    cleanup_ap_vendors(&ap_pool).await;

    // Start party server (not used in this test but ensures PARTY_MASTER_URL set)
    let _party_port = spawn_party_server(party_pool.clone()).await;

    let ap = make_ap_router(ap_pool.clone());

    let run_id = Uuid::new_v4();
    let create_body = json!({
        "name": format!("No Party Vendor {}", &run_id.to_string()[..8]),
        "currency": "USD",
        "payment_terms_days": 30
        // No party_id field
    });

    let (create_status, create_resp) = ap_send(
        &ap,
        "POST",
        "/api/ap/vendors",
        Some(create_body),
        true,
        AP_TENANT_ID,
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "Vendor without party_id must be accepted; body={}",
        create_resp
    );

    let vendor_id = create_resp["vendor_id"]
        .as_str()
        .expect("vendor_id in create response");

    // party_id field should be null in response
    assert!(
        create_resp["party_id"].is_null(),
        "party_id must be null when not provided; body={}",
        create_resp
    );

    // Verify via GET
    let (get_status, get_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        AP_TENANT_ID,
    )
    .await;

    assert_eq!(
        get_status,
        StatusCode::OK,
        "GET must return 200; body={}",
        get_resp
    );
    assert!(
        get_resp["party_id"].is_null(),
        "GET vendor party_id must be null when not set; body={}",
        get_resp
    );

    // Verify in DB
    let vendor_uuid = Uuid::parse_str(vendor_id).expect("vendor_id must be a UUID");
    let db_party_id: Option<Uuid> =
        sqlx::query_scalar("SELECT party_id FROM vendors WHERE vendor_id = $1 AND tenant_id = $2")
            .bind(vendor_uuid)
            .bind(AP_TENANT_ID)
            .fetch_one(&ap_pool)
            .await
            .expect("DB query failed");

    assert!(
        db_party_id.is_none(),
        "party_id must be NULL in DB when not provided; got {:?}",
        db_party_id
    );

    println!("✅ Vendor without party_id: party_id is NULL throughout (create, GET, DB)");

    // Cleanup
    cleanup_ap_vendors(&ap_pool).await;
}
