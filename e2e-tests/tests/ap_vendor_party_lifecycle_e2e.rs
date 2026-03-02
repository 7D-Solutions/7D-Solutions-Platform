//! E2E: AP + Party Master vendor lifecycle (bd-1e6g)
//!
//! Proves the AP vendor ↔ Party Master link works end-to-end through the
//! full vendor lifecycle: create → read → update → deactivate, with the
//! critical invariant that deactivating an AP vendor does NOT cascade to
//! the Party Master record.
//!
//! ## Invariants
//! - party_id stored on create round-trips correctly through GET
//! - party_id is updateable via PUT
//! - Deactivating a vendor returns 204
//! - Party Master record stays active after vendor deactivation
//! - Vendor is findable by party_id in DB
//!
//! ## Services required
//! - ap-postgres at localhost:5443
//! - party-postgres at localhost:5448
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- ap_vendor_party_lifecycle_e2e --nocapture
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

const LIFECYCLE_TENANT_ID: &str = "ap-party-lifecycle-test-tenant";

// ============================================================================
// Helpers
// ============================================================================

async fn run_ap_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ap/db/migrations")
        .run(pool)
        .await
        .expect("AP migrations failed");
}

async fn run_party_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

fn make_ap_router(pool: PgPool) -> Router {
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

async fn ap_send(
    router: &Router,
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
/// Sets PARTY_MASTER_URL env var so the AP service can reach it.
///
/// # Safety
/// Caller must hold the `serial` lock to prevent env var races.
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

async fn cleanup_ap_vendors(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' AND aggregate_id IN \
         (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
    )
    .bind(LIFECYCLE_TENANT_ID)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(LIFECYCLE_TENANT_ID)
        .execute(pool)
        .await
        .ok();
}

async fn cleanup_party(pool: &PgPool, party_id: Uuid) {
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(LIFECYCLE_TENANT_ID)
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

fn make_company_req(run_id: Uuid, suffix: &str) -> CreateCompanyRequest {
    CreateCompanyRequest {
        display_name: format!(
            "Lifecycle Vendor Corp{} {}",
            suffix,
            &run_id.to_string()[..8]
        ),
        legal_name: format!(
            "Lifecycle Vendor Corporation{} {}",
            suffix,
            &run_id.to_string()[..8]
        ),
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
    }
}

// ============================================================================
// Test: Full vendor lifecycle with party_id
// ============================================================================

/// Proves the full AP vendor ↔ Party Master lifecycle:
/// create party → create vendor → GET verifies party_id → PUT updates party_id
/// → deactivate vendor → party still active → DB party-based lookup works.
#[tokio::test]
#[serial]
async fn test_ap_vendor_party_lifecycle_full() {
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

    // ── Step 1: Create a Company party in Party Master ───────────────────
    let run_id = Uuid::new_v4();
    let party_view = party_service::create_company(
        &party_pool,
        LIFECYCLE_TENANT_ID,
        &make_company_req(run_id, ""),
        run_id.to_string(),
    )
    .await
    .expect("create_company failed");

    let party_id = party_view.party.id;
    assert_eq!(party_view.party.party_type, "company");
    assert_eq!(party_view.party.status, "active");
    println!("Step 1: Created party {} (status=active)", party_id);

    // ── Step 2: Create AP vendor referencing that party_id ───────────────
    let vendor_name = format!("Lifecycle Vendor {}", &run_id.to_string()[..8]);
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
        LIFECYCLE_TENANT_ID,
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
        .expect("vendor_id in create response")
        .to_string();

    let create_party_id = create_resp["party_id"]
        .as_str()
        .expect("party_id in create response");
    assert_eq!(
        create_party_id,
        party_id.to_string(),
        "create response party_id must match the party we created"
    );
    println!(
        "Step 2: Created vendor {} with party_id {}",
        vendor_id, create_party_id
    );

    // ── Step 3: GET vendor — verify party_id round-trips ─────────────────
    let (get_status, get_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        LIFECYCLE_TENANT_ID,
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
        "GET vendor party_id must match"
    );
    println!("Step 3: GET vendor party_id verified: {}", get_party_id);

    // ── Step 4: Update vendor — PUT with a second party ──────────────────
    let party2_view = party_service::create_company(
        &party_pool,
        LIFECYCLE_TENANT_ID,
        &make_company_req(run_id, "2"),
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
        LIFECYCLE_TENANT_ID,
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
        .expect("party_id in PUT response");
    assert_eq!(
        updated_party_id,
        party2_id.to_string(),
        "PUT response must reflect updated party_id"
    );
    println!("Step 4: Updated vendor party_id to {}", updated_party_id);

    // GET after update should reflect the new party_id
    let (get2_status, get2_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(
        get2_status,
        StatusCode::OK,
        "GET after update must return 200"
    );
    assert_eq!(
        get2_resp["party_id"]
            .as_str()
            .expect("party_id in GET after update"),
        party2_id.to_string(),
        "GET after update must reflect new party_id"
    );

    // ── Step 5: Deactivate vendor → 204 ──────────────────────────────────
    let (deactivate_status, _) = ap_send(
        &ap,
        "POST",
        &format!("/api/ap/vendors/{}/deactivate", vendor_id),
        None,
        true,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(
        deactivate_status,
        StatusCode::NO_CONTENT,
        "POST /api/ap/vendors/{}/deactivate must return 204",
        vendor_id
    );
    println!("Step 5: Vendor {} deactivated (204)", vendor_id);

    // Verify vendor is inactive via GET
    let (get3_status, get3_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(
        get3_status,
        StatusCode::OK,
        "GET deactivated vendor must return 200; body={}",
        get3_resp
    );
    assert!(
        !get3_resp["is_active"].as_bool().unwrap_or(true),
        "Deactivated vendor must have is_active=false; body={}",
        get3_resp
    );

    // ── Step 6: Verify party record is still active ───────────────────────
    // Deactivating an AP vendor must NOT cascade to the Party Master record.
    let party_status: String =
        sqlx::query_scalar("SELECT status::TEXT FROM party_parties WHERE id = $1")
            .bind(party_id)
            .fetch_one(&party_pool)
            .await
            .expect("party_parties query failed");

    assert_eq!(
        party_status, "active",
        "Party Master record must remain active after vendor deactivation; got '{}'",
        party_status
    );
    println!(
        "Step 6: Party {} is still '{}' after vendor deactivation — no cascade",
        party_id, party_status
    );

    // Also verify the second party (the one currently linked to the vendor) is active
    let party2_status: String =
        sqlx::query_scalar("SELECT status::TEXT FROM party_parties WHERE id = $1")
            .bind(party2_id)
            .fetch_one(&party_pool)
            .await
            .expect("party2_parties query failed");

    assert_eq!(
        party2_status, "active",
        "Second party must also remain active; got '{}'",
        party2_status
    );

    // ── Step 7: Verify vendor findable by party_id in DB ─────────────────
    let vendor_uuid = Uuid::parse_str(&vendor_id).expect("vendor_id must be a UUID");
    let linked_vendors: Vec<Uuid> =
        sqlx::query_scalar("SELECT vendor_id FROM vendors WHERE tenant_id = $1 AND party_id = $2")
            .bind(LIFECYCLE_TENANT_ID)
            .bind(party2_id)
            .fetch_all(&ap_pool)
            .await
            .expect("party-based vendor DB lookup failed");

    assert!(
        linked_vendors.contains(&vendor_uuid),
        "Vendor must be findable by party_id in DB; linked_vendors={:?}",
        linked_vendors
    );
    println!(
        "Step 7: DB party-based lookup: {} vendor(s) linked to party {}",
        linked_vendors.len(),
        party2_id
    );

    // ── Cleanup ────────────────────────────────────────────────────────────
    cleanup_ap_vendors(&ap_pool).await;
    cleanup_party(&party_pool, party_id).await;
    cleanup_party(&party_pool, party2_id).await;

    println!("✅ AP vendor + Party Master lifecycle: all 7 steps passed");
}

// ============================================================================
// Test: Deactivated vendor does NOT appear in default list
// ============================================================================

/// Proves that GET /api/ap/vendors (without include_inactive) excludes
/// deactivated vendors, while the party record remains untouched.
#[tokio::test]
#[serial]
async fn test_deactivated_vendor_excluded_from_default_list() {
    // ── Connect to databases ──────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = get_ap_pool().await;
    run_ap_migrations(&ap_pool).await;
    cleanup_ap_vendors(&ap_pool).await;

    let _party_port = spawn_party_server(party_pool.clone()).await;
    let ap = make_ap_router(ap_pool.clone());

    // Create party and vendor
    let run_id = Uuid::new_v4();
    let party_view = party_service::create_company(
        &party_pool,
        LIFECYCLE_TENANT_ID,
        &make_company_req(run_id, "-list"),
        run_id.to_string(),
    )
    .await
    .expect("create_company failed");
    let party_id = party_view.party.id;

    let create_body = json!({
        "name": format!("List Test Vendor {}", &run_id.to_string()[..8]),
        "currency": "USD",
        "payment_terms_days": 30,
        "party_id": party_id.to_string()
    });

    let (create_status, create_resp) = ap_send(
        &ap,
        "POST",
        "/api/ap/vendors",
        Some(create_body),
        true,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "create must return 201; body={}",
        create_resp
    );
    let vendor_id = create_resp["vendor_id"]
        .as_str()
        .expect("vendor_id in create response")
        .to_string();

    // Verify vendor appears in active list
    let (list_status, list_resp) = ap_send(
        &ap,
        "GET",
        "/api/ap/vendors",
        None,
        false,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(list_status, StatusCode::OK, "list must return 200");
    let vendors = list_resp.as_array().expect("list must return array");
    assert!(
        vendors
            .iter()
            .any(|v| v["vendor_id"].as_str() == Some(&vendor_id)),
        "Active vendor must appear in default list"
    );

    // Deactivate
    let (deact_status, _) = ap_send(
        &ap,
        "POST",
        &format!("/api/ap/vendors/{}/deactivate", vendor_id),
        None,
        true,
        LIFECYCLE_TENANT_ID,
    )
    .await;
    assert_eq!(
        deact_status,
        StatusCode::NO_CONTENT,
        "deactivate must return 204"
    );

    // Default list should NOT include the deactivated vendor
    let (list2_status, list2_resp) = ap_send(
        &ap,
        "GET",
        "/api/ap/vendors",
        None,
        false,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(
        list2_status,
        StatusCode::OK,
        "list after deactivation must return 200"
    );
    let vendors2 = list2_resp.as_array().expect("list must return array");
    assert!(
        !vendors2
            .iter()
            .any(|v| v["vendor_id"].as_str() == Some(&vendor_id)),
        "Deactivated vendor must NOT appear in default list (include_inactive=false)"
    );

    // include_inactive=true SHOULD include it
    let (list3_status, list3_resp) = ap_send(
        &ap,
        "GET",
        "/api/ap/vendors?include_inactive=true",
        None,
        false,
        LIFECYCLE_TENANT_ID,
    )
    .await;

    assert_eq!(list3_status, StatusCode::OK);
    let vendors3 = list3_resp.as_array().expect("list must return array");
    assert!(
        vendors3
            .iter()
            .any(|v| v["vendor_id"].as_str() == Some(&vendor_id)),
        "Deactivated vendor MUST appear when include_inactive=true"
    );

    // Party record remains active
    let party_status: String =
        sqlx::query_scalar("SELECT status::TEXT FROM party_parties WHERE id = $1")
            .bind(party_id)
            .fetch_one(&party_pool)
            .await
            .expect("party query failed");

    assert_eq!(
        party_status, "active",
        "Party must remain active after vendor deactivation"
    );
    println!(
        "✅ Party {} still active after vendor deactivation",
        party_id
    );

    // Cleanup
    cleanup_ap_vendors(&ap_pool).await;
    cleanup_party(&party_pool, party_id).await;

    println!("✅ Deactivated vendor excluded from default list: all assertions passed");
}
