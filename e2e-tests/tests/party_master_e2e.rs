//! E2E: Party Master CRUD + search — create, get, update, deactivate, search (bd-new9)
//!
//! **Coverage (7 test functions):**
//! 1. POST /api/party/companies — create company → 201 + party_id
//! 2. POST /api/party/individuals — create individual → 201 + party_id
//! 3. GET  /api/party/parties/:id — matches created company
//! 4. PUT  /api/party/parties/:id — update display_name reflected in GET
//! 5. POST /api/party/parties/:id/deactivate — party excluded from list
//! 6. GET  /api/party/parties/search?party_type=company — returns only companies
//! 7. GET  /api/party/parties/search?name=fragment — returns matching parties
//!
//! **Pattern:** In-process Axum router + real party-postgres (port 5448).
//! No Docker spin-up, no mocks, no stubs.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- party_master_e2e --nocapture
//! ```

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::get_party_pool;
use party_rs::{http, metrics::PartyMetrics, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Run party migrations idempotently.
async fn run_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

/// Build an in-process party router wired to the real pool, with test JWT layer.
fn make_router(pool: PgPool) -> axum::Router {
    let metrics = Arc::new(PartyMetrics::new().expect("metrics init failed"));
    let state = Arc::new(AppState { pool, metrics });
    common::with_test_jwt_layer(http::router(state))
}

/// Send a JSON request and return (status, parsed body).
async fn send_json(
    router: &axum::Router,
    method: &str,
    uri: &str,
    app_id: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let body_bytes = match body {
        Some(v) => v.to_string().into_bytes(),
        None => vec![],
    };
    let jwt = common::sign_test_jwt(app_id, &["party.mutate", "party.read"]);
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-app-id", app_id)
        .header("x-actor-id", "test-actor")
        .header("x-correlation-id", Uuid::new_v4().to_string())
        .header("authorization", format!("Bearer {}", jwt));
    if !body_bytes.is_empty() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder.body(Body::from(body_bytes)).unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let resp_bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&resp_bytes).unwrap_or(json!({}));
    (status, parsed)
}

/// Clean up all party rows created by this test's app_id.
async fn cleanup(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_external_refs WHERE party_id IN (SELECT id FROM party_parties WHERE app_id = $1)")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_companies WHERE party_id IN (SELECT id FROM party_parties WHERE app_id = $1)")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_individuals WHERE party_id IN (SELECT id FROM party_parties WHERE app_id = $1)")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_parties WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: Create company party → 201 + party_id
// ============================================================================

#[tokio::test]
async fn test_create_company_returns_201() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    let (status, body) = send_json(
        &router,
        "POST",
        "/api/party/companies",
        &app_id,
        Some(json!({
            "display_name": "Acme Corp E2E",
            "legal_name": "Acme Corporation Ltd",
            "industry_code": "TECH",
            "email": "contact@acme.example"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "expected 201; body={}", body);
    let party_id = body["party"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .expect("response must contain party id");
    assert!(!party_id.is_empty(), "party_id must not be empty");

    // Verify the party type in response
    let party_type = body["party"]["party_type"]
        .as_str()
        .or_else(|| body["party_type"].as_str())
        .unwrap_or("");
    assert_eq!(
        party_type, "company",
        "party_type must be 'company'; body={}",
        body
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Test 2: Create individual party → 201 + party_id
// ============================================================================

#[tokio::test]
async fn test_create_individual_returns_201() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    let (status, body) = send_json(
        &router,
        "POST",
        "/api/party/individuals",
        &app_id,
        Some(json!({
            "display_name": "Jane Smith",
            "first_name": "Jane",
            "last_name": "Smith",
            "email": "jane.smith@example.com",
            "job_title": "Engineer"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "expected 201; body={}", body);
    let party_id = body["party"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .expect("response must contain party id");
    assert!(!party_id.is_empty(), "party_id must not be empty");

    let party_type = body["party"]["party_type"]
        .as_str()
        .or_else(|| body["party_type"].as_str())
        .unwrap_or("");
    assert_eq!(
        party_type, "individual",
        "party_type must be 'individual'; body={}",
        body
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Test 3: GET /api/party/parties/:id — matches created party
// ============================================================================

#[tokio::test]
async fn test_get_party_matches_created() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    // Create company
    let (create_status, create_body) = send_json(
        &router,
        "POST",
        "/api/party/companies",
        &app_id,
        Some(json!({
            "display_name": "GetTest Corp",
            "legal_name": "GetTest Corporation"
        })),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "create failed; body={}",
        create_body
    );

    let party_id = create_body["party"]["id"]
        .as_str()
        .or_else(|| create_body["id"].as_str())
        .expect("party_id in create response");

    // Fetch by ID
    let (get_status, get_body) = send_json(
        &router,
        "GET",
        &format!("/api/party/parties/{}", party_id),
        &app_id,
        None,
    )
    .await;

    assert_eq!(
        get_status,
        StatusCode::OK,
        "expected 200; body={}",
        get_body
    );

    let fetched_id = get_body["party"]["id"]
        .as_str()
        .or_else(|| get_body["id"].as_str())
        .unwrap_or("");
    assert_eq!(
        fetched_id, party_id,
        "fetched id must match created id; body={}",
        get_body
    );

    let fetched_name = get_body["party"]["display_name"]
        .as_str()
        .or_else(|| get_body["display_name"].as_str())
        .unwrap_or("");
    assert_eq!(
        fetched_name, "GetTest Corp",
        "display_name must match; body={}",
        get_body
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Test 4: PUT /api/party/parties/:id — update display_name reflected in GET
// ============================================================================

#[tokio::test]
async fn test_update_party_display_name() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    // Create individual
    let (create_status, create_body) = send_json(
        &router,
        "POST",
        "/api/party/individuals",
        &app_id,
        Some(json!({
            "display_name": "Old Name",
            "first_name": "Old",
            "last_name": "Name"
        })),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "create failed; body={}",
        create_body
    );

    let party_id = create_body["party"]["id"]
        .as_str()
        .or_else(|| create_body["id"].as_str())
        .expect("party_id in create response");

    // Update display_name
    let (update_status, update_body) = send_json(
        &router,
        "PUT",
        &format!("/api/party/parties/{}", party_id),
        &app_id,
        Some(json!({
            "display_name": "Updated Name"
        })),
    )
    .await;
    assert_eq!(
        update_status,
        StatusCode::OK,
        "update failed; body={}",
        update_body
    );

    // Verify via GET
    let (get_status, get_body) = send_json(
        &router,
        "GET",
        &format!("/api/party/parties/{}", party_id),
        &app_id,
        None,
    )
    .await;
    assert_eq!(
        get_status,
        StatusCode::OK,
        "get after update failed; body={}",
        get_body
    );

    let updated_name = get_body["party"]["display_name"]
        .as_str()
        .or_else(|| get_body["display_name"].as_str())
        .unwrap_or("");
    assert_eq!(
        updated_name, "Updated Name",
        "display_name must reflect update; body={}",
        get_body
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Test 5: Deactivate → excluded from list
// ============================================================================

#[tokio::test]
async fn test_deactivate_excluded_from_list() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    // Create company
    let (create_status, create_body) = send_json(
        &router,
        "POST",
        "/api/party/companies",
        &app_id,
        Some(json!({
            "display_name": "DeactivateMe Inc",
            "legal_name": "DeactivateMe Incorporated"
        })),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "create failed; body={}",
        create_body
    );

    let party_id = create_body["party"]["id"]
        .as_str()
        .or_else(|| create_body["id"].as_str())
        .expect("party_id in create response");

    // Deactivate
    let (deact_status, deact_body) = send_json(
        &router,
        "POST",
        &format!("/api/party/parties/{}/deactivate", party_id),
        &app_id,
        None,
    )
    .await;
    assert!(
        deact_status == StatusCode::NO_CONTENT || deact_status == StatusCode::OK,
        "deactivate must return 204 or 200; got {}; body={}",
        deact_status,
        deact_body
    );

    // List active parties — deactivated party must be absent
    let (list_status, list_body) =
        send_json(&router, "GET", "/api/party/parties", &app_id, None).await;
    assert_eq!(
        list_status,
        StatusCode::OK,
        "list failed; body={}",
        list_body
    );

    let parties = list_body
        .as_array()
        .expect("list response must be an array");
    let found = parties.iter().any(|p| p["id"].as_str() == Some(party_id));
    assert!(
        !found,
        "deactivated party must not appear in default list; party_id={}, list={}",
        party_id, list_body
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Test 6: Search by party_type=company → only companies returned
// ============================================================================

#[tokio::test]
async fn test_search_by_type_returns_only_companies() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    // Create one company
    let (s1, b1) = send_json(
        &router,
        "POST",
        "/api/party/companies",
        &app_id,
        Some(json!({
            "display_name": "TypeFilter Corp",
            "legal_name": "TypeFilter Corporation"
        })),
    )
    .await;
    assert_eq!(
        s1,
        StatusCode::CREATED,
        "company create failed; body={}",
        b1
    );

    // Create one individual (should be excluded from company filter)
    let (s2, b2) = send_json(
        &router,
        "POST",
        "/api/party/individuals",
        &app_id,
        Some(json!({
            "display_name": "TypeFilter Person",
            "first_name": "TypeFilter",
            "last_name": "Person"
        })),
    )
    .await;
    assert_eq!(
        s2,
        StatusCode::CREATED,
        "individual create failed; body={}",
        b2
    );

    // Search by type=company
    let (search_status, search_body) = send_json(
        &router,
        "GET",
        "/api/party/parties/search?party_type=company",
        &app_id,
        None,
    )
    .await;
    assert_eq!(
        search_status,
        StatusCode::OK,
        "search failed; body={}",
        search_body
    );

    let results = search_body
        .as_array()
        .expect("search response must be array");
    // All results must be company type
    for p in results.iter() {
        let pt = p["party_type"].as_str().unwrap_or("");
        assert_eq!(
            pt, "company",
            "all search results must be 'company' type; got party={}",
            p
        );
    }
    // Must include our created company
    let company_id = b1["party"]["id"]
        .as_str()
        .or_else(|| b1["id"].as_str())
        .unwrap_or("");
    let found_company = results.iter().any(|p| p["id"].as_str() == Some(company_id));
    assert!(
        found_company,
        "search must include created company; company_id={}",
        company_id
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Test 7: Search by name fragment → matching parties returned
// ============================================================================

#[tokio::test]
async fn test_search_by_name_fragment() {
    let pool = get_party_pool().await;
    run_migrations(&pool).await;

    // Use a unique fragment to avoid cross-test contamination
    let fragment = format!("ZXQ{}", &Uuid::new_v4().simple().to_string()[..6]);
    let app_id = Uuid::new_v4().to_string();
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());

    // Create a party whose display_name contains the fragment
    let display_name = format!("{} Industries", fragment);
    let (create_status, create_body) = send_json(
        &router,
        "POST",
        "/api/party/companies",
        &app_id,
        Some(json!({
            "display_name": display_name,
            "legal_name": format!("{} Industries Ltd", fragment)
        })),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "create failed; body={}",
        create_body
    );

    let party_id = create_body["party"]["id"]
        .as_str()
        .or_else(|| create_body["id"].as_str())
        .expect("party_id in create response");

    // Create a decoy party that does NOT contain the fragment
    let (decoy_status, decoy_body) = send_json(
        &router,
        "POST",
        "/api/party/individuals",
        &app_id,
        Some(json!({
            "display_name": "Completely Unrelated Party",
            "first_name": "Decoy",
            "last_name": "Person"
        })),
    )
    .await;
    assert_eq!(
        decoy_status,
        StatusCode::CREATED,
        "decoy create failed; body={}",
        decoy_body
    );

    // Search by name fragment (alphanumeric only — no encoding needed)
    let search_uri = format!("/api/party/parties/search?name={}", fragment);
    let (search_status, search_body) = send_json(&router, "GET", &search_uri, &app_id, None).await;
    assert_eq!(
        search_status,
        StatusCode::OK,
        "search failed; body={}",
        search_body
    );

    let results = search_body
        .as_array()
        .expect("search response must be array");
    assert!(
        !results.is_empty(),
        "search must return at least one result; fragment={}",
        fragment
    );

    // Our party must appear in results
    let found = results.iter().any(|p| p["id"].as_str() == Some(party_id));
    assert!(
        found,
        "search must include party matching fragment '{}'; party_id={}; results={}",
        fragment, party_id, search_body
    );

    // All results must contain the fragment in display_name (case-insensitive)
    let lower_fragment = fragment.to_lowercase();
    for p in results.iter() {
        let name = p["display_name"].as_str().unwrap_or("").to_lowercase();
        assert!(
            name.contains(&lower_fragment),
            "all results must match fragment '{}'; got display_name='{}'",
            fragment,
            name
        );
    }

    cleanup(&pool, &app_id).await;
}
