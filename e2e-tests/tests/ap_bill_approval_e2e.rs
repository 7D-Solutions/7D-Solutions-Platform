//! E2E: AP bill approval state machine — submit, approve, reject (void) paths (bd-1e5j)
//!
//! Proves the AP bill approval state machine for both approval and rejection paths:
//!
//! Happy path:
//! 1. Create AP vendor + submit bill via domain layer.
//! 2. Approve via HTTP: POST /api/ap/bills/{bill_id}/approve → status=approved
//! 3. Verify approved bill appears in the non-voided list (payment run candidates use status='approved').
//!
//! Rejection path:
//! 4. Create a second bill via domain layer.
//! 5. Void (reject) via HTTP: POST /api/ap/bills/{bill_id}/void { reason: "duplicate_invoice" }
//! 6. Verify voided bill is excluded from the default bill list (payment run candidates).
//! 7. Attempt to approve the voided bill → HTTP 422 (state machine guard).
//!
//! Note: The AP module uses "void" as the bill rejection mechanism. The state machine
//! guards: `open | matched → approved` and `open | matched | approved → voided`.
//! Voided bills cannot be approved (InvalidTransition → HTTP 422).
//!
//! ## Services required
//! - ap-postgres at localhost:5443
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- ap_bill_approval_e2e --nocapture
//! ```

mod common;

use ap::{
    domain::{
        bills::{
            service::create_bill, ApproveBillRequest, CreateBillLineRequest, CreateBillRequest,
            VoidBillRequest,
        },
        tax::ZeroTaxProvider,
    },
    http,
    metrics::ApMetrics,
    AppState,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use common::get_ap_pool;
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

const TENANT_ID: &str = "ap-bill-approval-e2e-tenant";

// ============================================================================
// Router helpers
// ============================================================================

fn make_ap_router(pool: PgPool) -> Router {
    let metrics = Arc::new(ApMetrics::new().expect("AP metrics init failed"));
    let state = Arc::new(AppState { pool, metrics });

    let ap_mutations = Router::new()
        .route("/api/ap/bills/{bill_id}/approve", post(http::bills::approve_bill))
        .route("/api/ap/bills/{bill_id}/void", post(http::bills::void_bill))
        .route_layer(RequirePermissionsLayer::new(&[permissions::AP_MUTATE]))
        .with_state(state.clone());

    Router::new()
        .route("/api/ap/bills", get(http::bills::list_bills))
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
) -> (StatusCode, Value) {
    let body_bytes = match &body {
        Some(v) => v.to_string().into_bytes(),
        None => vec![],
    };

    let mut builder = Request::builder().method(method).uri(uri);
    builder = builder.header("x-tenant-id", TENANT_ID);
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
    let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, parsed)
}

// ============================================================================
// Setup helpers
// ============================================================================

async fn run_ap_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ap/db/migrations")
        .run(pool)
        .await
        .expect("AP migrations failed");
}

/// Insert a minimal vendor directly into the DB.
async fn create_vendor_db(pool: &PgPool) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
         is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
    )
    .bind(vendor_id)
    .bind(TENANT_ID)
    .bind(format!("Approval-E2E-Vendor-{}", &vendor_id.to_string()[..8]))
    .execute(pool)
    .await
    .expect("insert vendor");
    vendor_id
}

/// Create a bill via the domain layer and return its bill_id.
async fn create_bill_domain(
    pool: &PgPool,
    vendor_id: Uuid,
    inv_ref: &str,
    amount_cents: i64,
) -> Uuid {
    let bill = create_bill(
        pool,
        TENANT_ID,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: inv_ref.to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-30T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            tax_minor: None,
            entered_by: "ap-clerk-e2e".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("E2E services".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: amount_cents,
                gl_account_code: Some("6100".to_string()),
                po_line_id: None,
            }],
        },
        format!("corr-create-{}", inv_ref),
    )
    .await
    .expect("create_bill failed");

    assert_eq!(bill.bill.status, "open", "new bill must start as open");
    bill.bill.bill_id
}

/// Clean all test data for TENANT_ID.
async fn cleanup(pool: &PgPool) {
    for q in [
        "DELETE FROM payment_run_executions WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_run_items WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_runs WHERE tenant_id = $1",
        "DELETE FROM ap_allocations WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM three_way_match WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
         AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
         AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TENANT_ID).execute(pool).await.ok();
    }
}

// ============================================================================
// Test 1: Happy path — bill approved, appears in payment run candidate list
// ============================================================================

/// Approve an open bill via HTTP and verify the state machine transitions correctly.
///
/// Steps:
/// 1. Create vendor + bill (domain layer).
/// 2. POST /api/ap/bills/{bill_id}/approve (with override_reason since bill is unmatched).
/// 3. Verify HTTP 200 and status=approved.
/// 4. Verify GET /api/ap/bills includes the approved bill (not filtered out).
/// 5. Verify DB status=approved.
/// 6. Verify ap.vendor_bill_approved event in outbox.
#[tokio::test]
#[serial]
async fn test_approve_bill_happy_path() {
    let pool = get_ap_pool().await;
    run_ap_migrations(&pool).await;
    cleanup(&pool).await;

    let vendor_id = create_vendor_db(&pool).await;
    let bill_id = create_bill_domain(&pool, vendor_id, "INV-APPROVE-001", 75_000).await;

    let router = make_ap_router(pool.clone());

    // ── Step 2: Approve via HTTP ─────────────────────────────────────────────
    let (approve_status, approve_resp) = ap_send(
        &router,
        "POST",
        &format!("/api/ap/bills/{}/approve", bill_id),
        Some(json!({
            "approved_by": "controller-e2e",
            "override_reason": "spot purchase, no PO required"
        })),
        true,
    )
    .await;

    assert_eq!(
        approve_status,
        StatusCode::OK,
        "POST /api/ap/bills/{}/approve must return 200; body={}",
        bill_id,
        approve_resp
    );
    assert_eq!(
        approve_resp["status"].as_str(),
        Some("approved"),
        "approve response must have status=approved; body={}",
        approve_resp
    );
    println!("✓ Bill {} approved (HTTP 200, status=approved)", bill_id);

    // ── Step 3: Verify bill appears in default list (payment run candidates) ─
    let (list_status, list_resp) = ap_send(&router, "GET", "/api/ap/bills", None, false).await;

    assert_eq!(list_status, StatusCode::OK, "GET /api/ap/bills must return 200");
    let bills = list_resp.as_array().expect("list response must be array");
    let approved_in_list = bills.iter().any(|b| {
        b["bill_id"].as_str() == Some(&bill_id.to_string())
            && b["status"].as_str() == Some("approved")
    });
    assert!(
        approved_in_list,
        "approved bill must appear in default GET /api/ap/bills list; got: {}",
        list_resp
    );
    println!("✓ Approved bill {} appears in payment run candidate list", bill_id);

    // ── Step 4: Verify DB status ─────────────────────────────────────────────
    let (db_status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill_id)
            .bind(TENANT_ID)
            .fetch_one(&pool)
            .await
            .expect("fetch bill status from DB");
    assert_eq!(db_status, "approved", "DB must show status=approved");
    println!("✓ DB status=approved confirmed for bill {}", bill_id);

    // ── Step 5: Verify outbox event ──────────────────────────────────────────
    let (event_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.vendor_bill_approved' AND aggregate_id = $1",
    )
    .bind(bill_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("fetch outbox event count");
    assert_eq!(event_count, 1, "ap.vendor_bill_approved event must be in outbox");
    println!("✓ ap.vendor_bill_approved event in outbox for bill {}", bill_id);

    cleanup(&pool).await;
    println!("✅ test_approve_bill_happy_path PASS");
}

// ============================================================================
// Test 2: Rejection (void) path — bill voided, excluded from payment run candidates
// ============================================================================

/// Void a bill (rejection path) and verify it is excluded from payment run candidates.
///
/// Steps:
/// 1. Create vendor + bill (domain layer).
/// 2. POST /api/ap/bills/{bill_id}/void { void_reason: "duplicate_invoice" }
/// 3. Verify HTTP 200 and status=voided.
/// 4. Verify voided bill NOT in default GET /api/ap/bills (excluded from payment runs).
/// 5. Attempt POST /api/ap/bills/{bill_id}/approve → must return 422 (state machine guard).
#[tokio::test]
#[serial]
async fn test_reject_bill_void_path() {
    let pool = get_ap_pool().await;
    run_ap_migrations(&pool).await;
    cleanup(&pool).await;

    let vendor_id = create_vendor_db(&pool).await;
    let bill_id = create_bill_domain(&pool, vendor_id, "INV-REJECT-001", 50_000).await;

    let router = make_ap_router(pool.clone());

    // ── Step 2: Void (reject) via HTTP ───────────────────────────────────────
    let (void_status, void_resp) = ap_send(
        &router,
        "POST",
        &format!("/api/ap/bills/{}/void", bill_id),
        Some(json!({
            "voided_by": "ap-reviewer-e2e",
            "void_reason": "duplicate_invoice"
        })),
        true,
    )
    .await;

    assert_eq!(
        void_status,
        StatusCode::OK,
        "POST /api/ap/bills/{}/void must return 200; body={}",
        bill_id,
        void_resp
    );
    assert_eq!(
        void_resp["status"].as_str(),
        Some("voided"),
        "void response must have status=voided; body={}",
        void_resp
    );
    println!("✓ Bill {} voided (HTTP 200, status=voided)", bill_id);

    // ── Step 3: Verify voided bill NOT in default list ───────────────────────
    let (list_status, list_resp) = ap_send(&router, "GET", "/api/ap/bills", None, false).await;

    assert_eq!(list_status, StatusCode::OK, "GET /api/ap/bills must return 200");
    let bills = list_resp.as_array().expect("list response must be array");
    let voided_in_list = bills
        .iter()
        .any(|b| b["bill_id"].as_str() == Some(&bill_id.to_string()));
    assert!(
        !voided_in_list,
        "voided bill must NOT appear in default GET /api/ap/bills (excluded from payment run candidates); got: {}",
        list_resp
    );
    println!("✓ Voided bill {} excluded from payment run candidate list", bill_id);

    // ── Step 4: Attempt to approve a voided bill → must fail 422 ────────────
    let (bad_approve_status, bad_approve_resp) = ap_send(
        &router,
        "POST",
        &format!("/api/ap/bills/{}/approve", bill_id),
        Some(json!({
            "approved_by": "controller-e2e",
            "override_reason": "trying to approve voided bill"
        })),
        true,
    )
    .await;

    assert_eq!(
        bad_approve_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "Approving a voided bill must return 422; body={}",
        bad_approve_resp
    );
    assert_eq!(
        bad_approve_resp["error"].as_str(),
        Some("invalid_transition"),
        "Error code must be invalid_transition; body={}",
        bad_approve_resp
    );
    println!(
        "✓ Approving voided bill {} correctly returns 422 invalid_transition",
        bill_id
    );

    cleanup(&pool).await;
    println!("✅ test_reject_bill_void_path PASS");
}

// ============================================================================
// Test 3: State machine guard — cannot approve an already-approved bill's sibling
//         that was rejected; also confirm payment run DB query excludes voided
// ============================================================================

/// Full state machine proof: approved bill in DB query for payment runs,
/// voided bill absent from same query.
///
/// Steps:
/// 1. Create vendor + two bills.
/// 2. Approve bill_a → status=approved.
/// 3. Void bill_b → status=voided.
/// 4. Direct DB query (mirrors payment run selector): count approved bills.
///    - bill_a must be counted (status IN ('approved', 'partially_paid')).
///    - bill_b must NOT be counted.
/// 5. Approve-after-void attempt on bill_b → 422.
#[tokio::test]
#[serial]
async fn test_payment_run_candidate_filtering() {
    let pool = get_ap_pool().await;
    run_ap_migrations(&pool).await;
    cleanup(&pool).await;

    let vendor_id = create_vendor_db(&pool).await;
    let bill_a = create_bill_domain(&pool, vendor_id, "INV-CAND-A", 80_000).await;
    let bill_b = create_bill_domain(&pool, vendor_id, "INV-CAND-B", 40_000).await;

    let router = make_ap_router(pool.clone());

    // Approve bill_a
    let (s, b) = ap_send(
        &router,
        "POST",
        &format!("/api/ap/bills/{}/approve", bill_a),
        Some(json!({ "approved_by": "controller-e2e", "override_reason": "no PO" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "approve bill_a failed; body={}", b);

    // Void bill_b
    let (s, b) = ap_send(
        &router,
        "POST",
        &format!("/api/ap/bills/{}/void", bill_b),
        Some(json!({ "voided_by": "reviewer-e2e", "void_reason": "duplicate_invoice" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "void bill_b failed; body={}", b);

    // ── DB query mirrors payment run candidate selector ───────────────────────
    // Payment runs use: status IN ('approved', 'partially_paid')
    let (approved_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vendor_bills \
         WHERE tenant_id = $1 AND status IN ('approved', 'partially_paid')",
    )
    .bind(TENANT_ID)
    .fetch_one(&pool)
    .await
    .expect("candidate count query");

    assert_eq!(
        approved_count, 1,
        "exactly 1 bill should be a payment run candidate (bill_a approved, bill_b voided)"
    );

    let (candidate_id,): (Uuid,) = sqlx::query_as(
        "SELECT bill_id FROM vendor_bills \
         WHERE tenant_id = $1 AND status IN ('approved', 'partially_paid') LIMIT 1",
    )
    .bind(TENANT_ID)
    .fetch_one(&pool)
    .await
    .expect("fetch candidate bill_id");

    assert_eq!(
        candidate_id, bill_a,
        "the candidate must be bill_a (the approved one)"
    );
    println!("✓ Payment run DB query: bill_a={} is candidate, bill_b={} excluded", bill_a, bill_b);

    // ── Attempt to approve the voided bill → 422 ─────────────────────────────
    let (s, b) = ap_send(
        &router,
        "POST",
        &format!("/api/ap/bills/{}/approve", bill_b),
        Some(json!({ "approved_by": "controller-e2e", "override_reason": "should fail" })),
        true,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "Approving voided bill must return 422; body={}",
        b
    );
    assert_eq!(
        b["error"].as_str(),
        Some("invalid_transition"),
        "error code must be invalid_transition; body={}",
        b
    );
    println!("✓ Voided bill_b cannot be approved (422 invalid_transition)");

    cleanup(&pool).await;
    println!("✅ test_payment_run_candidate_filtering PASS");
}
