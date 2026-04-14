//! E2E: GL Period Close Enforcement — AP/AR 422 on backdated entries
//!
//! Acceptance criteria (bd-w7kc5):
//! 1. Close a GL accounting period.
//! 2. POST AP bill with `invoice_date` in that period → 422 PERIOD_CLOSED.
//! 3. POST AR invoice with `billing_period_start` in that period → 422 PERIOD_CLOSED.
//! 4. Reopen the period.
//! 5. POST AP bill again → 201 Created.
//! 6. POST AR invoice again → 201 Created.
//!
//! All three databases (GL, AP, AR) are real running services.
//! No mocks, no stubs.
//!
//! Belt-and-suspenders note: GL's own posting consumer also enforces period
//! closure. That layer is tested in gl_outbox_atomicity_e2e.rs and smoke_gl_periods.rs.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests period_close_enforcement_e2e -- --nocapture
//! ```

mod common;

use ap::{http::bills::create_bill as ap_create_bill, metrics::ApMetrics, AppState};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Extension, Router,
};
use chrono::Utc;
use common::{get_ap_pool, get_ar_pool, get_gl_pool};
use security::{ActorType, VerifiedClaims};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Constants — January 2026, a past period safe to use for testing
// ============================================================================

const PERIOD_START: &str = "2026-01-01";
const PERIOD_END: &str = "2026-01-31";
/// An invoice/bill date inside the test period.
const IN_PERIOD_DATE: &str = "2026-01-15";

// ============================================================================
// DB helpers
// ============================================================================

async fn run_gl_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/gl/db/migrations")
        .run(pool)
        .await
        .expect("GL migrations failed");
}

async fn run_ap_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ap/db/migrations")
        .run(pool)
        .await
        .expect("AP migrations failed");
}

async fn run_ar_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ar/db/migrations")
        .run(pool)
        .await
        .expect("AR migrations failed");
}

/// Insert a test GL accounting period (open by default).
async fn seed_gl_period(gl_pool: &PgPool, tenant_id: &str) -> Uuid {
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end)
        VALUES ($1, $2, $3::DATE, $4::DATE)
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(PERIOD_START)
    .bind(PERIOD_END)
    .execute(gl_pool)
    .await
    .expect("seed GL accounting period");
    period_id
}

/// Close the period by setting closed_at + close_hash (constraint requires both).
async fn close_gl_period(gl_pool: &PgPool, period_id: Uuid) {
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = NOW(), closed_by = 'e2e-period-close-test', close_hash = 'e2e-test-hash-period-close-enforcement'
        WHERE id = $1
        "#,
    )
    .bind(period_id)
    .execute(gl_pool)
    .await
    .expect("close GL period");
}

/// Reopen the period by clearing closed_at and close_hash.
async fn reopen_gl_period(gl_pool: &PgPool, period_id: Uuid) {
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = NULL, closed_by = NULL, close_hash = NULL
        WHERE id = $1
        "#,
    )
    .bind(period_id)
    .execute(gl_pool)
    .await
    .expect("reopen GL period");
}

/// Insert a minimal AP vendor and return its vendor_id.
async fn seed_ap_vendor(ap_pool: &PgPool, tenant_id: &str) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
         is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(format!("PeriodTest-Vendor-{}", &vendor_id.to_string()[..8]))
    .execute(ap_pool)
    .await
    .expect("seed AP vendor");
    vendor_id
}

/// Insert a minimal AR customer and return its integer id.
async fn seed_ar_customer(ar_pool: &PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at) \
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW()) RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("period-test-{}@example.com", Uuid::new_v4()))
    .bind(format!("PeriodTest-Customer-{}", tenant_id))
    .fetch_one(ar_pool)
    .await
    .expect("seed AR customer")
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup(gl_pool: &PgPool, ap_pool: &PgPool, ar_pool: &PgPool, tenant_id: &str) {
    // GL
    for q in [
        "DELETE FROM close_checklist_items WHERE tenant_id = $1",
        "DELETE FROM period_reopen_requests WHERE tenant_id = $1",
        "DELETE FROM close_approvals WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(gl_pool).await.ok();
    }

    // AP (reverse FK order)
    for q in [
        "DELETE FROM bill_lines WHERE bill_id IN (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_type = 'bill' AND aggregate_id IN \
         (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' AND aggregate_id IN \
         (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(ap_pool).await.ok();
    }

    // AR (reverse FK order)
    for q in [
        "DELETE FROM ar_invoice_line_items WHERE app_id = $1",
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM ar_invoices WHERE app_id = $1",
        "DELETE FROM ar_customers WHERE app_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(ar_pool).await.ok();
    }
}

// ============================================================================
// In-process router builders
// ============================================================================

/// AP router wired with a real GL pool for period pre-validation.
fn make_ap_router(ap_pool: PgPool, gl_pool: PgPool) -> Router {
    let metrics = Arc::new(ApMetrics::new().expect("AP metrics init failed"));
    let state = Arc::new(AppState {
        pool: ap_pool,
        metrics,
        gl_pool: Some(gl_pool),
    });
    Router::new()
        .route("/api/ap/bills", post(ap_create_bill))
        .with_state(state)
}

/// AR router wired with a real GL pool via Extension layer.
fn make_ar_router(ar_pool: PgPool, gl_pool: PgPool) -> Router {
    ar_rs::http::ar_router_permissive(ar_pool).layer(Extension(Arc::new(gl_pool)))
}

// ============================================================================
// VerifiedClaims factory for injecting into request extensions
// ============================================================================

fn make_claims(tenant_id: &str) -> VerifiedClaims {
    let tid = Uuid::parse_str(tenant_id)
        .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, tenant_id.as_bytes()));
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: tid,
        app_id: Some(tid),
        roles: vec!["operator".to_string()],
        perms: vec!["ap.mutate".to_string(), "ar.mutate".to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "test".to_string(),
    }
}

// ============================================================================
// Request builders
// ============================================================================

/// Build a POST /api/ap/bills request with invoice_date in the closed period.
fn ap_create_bill_request(
    tenant_id: &str,
    vendor_id: Uuid,
    inv_ref: &str,
) -> Request<Body> {
    let body = json!({
        "vendor_id": vendor_id,
        "vendor_invoice_ref": inv_ref,
        "currency": "USD",
        "invoice_date": format!("{}T00:00:00Z", IN_PERIOD_DATE),
        "entered_by": "period-close-e2e",
        "lines": [{
            "description": "E2E test service",
            "quantity": 1.0,
            "unit_price_minor": 10000,
            "gl_account_code": "6100"
        }]
    });

    let mut req = Request::builder()
        .method("POST")
        .uri("/api/ap/bills")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build AP create bill request");

    req.extensions_mut().insert(make_claims(tenant_id));
    req
}

/// Build a POST /api/ar/invoices request with billing_period_start in the closed period.
fn ar_create_invoice_request(tenant_id: &str, ar_customer_id: i32) -> Request<Body> {
    let body = json!({
        "ar_customer_id": ar_customer_id,
        "amount_cents": 9900,
        "currency": "USD",
        "billing_period_start": format!("{}T00:00:00", IN_PERIOD_DATE),
        "billing_period_end": format!("{}T23:59:59", IN_PERIOD_DATE)
    });

    let mut req = Request::builder()
        .method("POST")
        .uri("/api/ar/invoices")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build AR create invoice request");

    req.extensions_mut().insert(make_claims(tenant_id));
    req
}

// ============================================================================
// Response parser
// ============================================================================

async fn parse_response(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 256 * 1024)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, body)
}

// ============================================================================
// Test
// ============================================================================

/// Verifies that AP bill creation and AR invoice creation both return 422
/// PERIOD_CLOSED when the GL period is closed, and 201 after reopening.
#[tokio::test]
async fn test_period_close_enforcement() {
    let gl_pool = get_gl_pool().await;
    let ap_pool = get_ap_pool().await;
    let ar_pool = get_ar_pool().await;

    run_gl_migrations(&gl_pool).await;
    run_ap_migrations(&ap_pool).await;
    run_ar_migrations(&ar_pool).await;

    // Unique tenant for test isolation
    let tenant_id = Uuid::new_v4().to_string();

    // Seed test data
    let period_id = seed_gl_period(&gl_pool, &tenant_id).await;
    let vendor_id = seed_ap_vendor(&ap_pool, &tenant_id).await;
    let ar_customer_id = seed_ar_customer(&ar_pool, &tenant_id).await;

    println!("tenant={} period={} vendor={} ar_customer={}", tenant_id, period_id, vendor_id, ar_customer_id);

    // ── Phase 1: Close the GL period ─────────────────────────────────────────
    close_gl_period(&gl_pool, period_id).await;
    println!("GL period {} closed", period_id);

    // ── Phase 2: AP bill → must return 422 ───────────────────────────────────
    {
        let ap_router = make_ap_router(ap_pool.clone(), gl_pool.clone());
        let req = ap_create_bill_request(&tenant_id, vendor_id, "PERIOD-TEST-CLOSED-001");
        let (status, body) = parse_response(ap_router.oneshot(req).await.unwrap()).await;

        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "AP: POST /api/ap/bills must return 422 for closed period; body={}",
            body
        );
        assert_eq!(
            body["error"].as_str().unwrap_or(""),
            "PERIOD_CLOSED",
            "AP: error code must be PERIOD_CLOSED; body={}",
            body
        );
        println!("AP: closed period correctly returns 422 PERIOD_CLOSED ({})", status);
    }

    // ── Phase 3: AR invoice → must return 422 ────────────────────────────────
    {
        let ar_router = make_ar_router(ar_pool.clone(), gl_pool.clone());
        let req = ar_create_invoice_request(&tenant_id, ar_customer_id);
        let (status, body) = parse_response(ar_router.oneshot(req).await.unwrap()).await;

        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "AR: POST /api/ar/invoices must return 422 for closed period; body={}",
            body
        );
        assert_eq!(
            body["error"].as_str().unwrap_or(""),
            "PERIOD_CLOSED",
            "AR: error code must be PERIOD_CLOSED; body={}",
            body
        );
        println!("AR: closed period correctly returns 422 PERIOD_CLOSED ({})", status);
    }

    // ── Phase 4: Reopen the GL period ────────────────────────────────────────
    reopen_gl_period(&gl_pool, period_id).await;
    println!("GL period {} reopened", period_id);

    // ── Phase 5: AP bill → must return 201 ───────────────────────────────────
    {
        let ap_router = make_ap_router(ap_pool.clone(), gl_pool.clone());
        // Use a different invoice ref (unique constraint per vendor/tenant)
        let req = ap_create_bill_request(&tenant_id, vendor_id, "PERIOD-TEST-OPEN-001");
        let (status, body) = parse_response(ap_router.oneshot(req).await.unwrap()).await;

        assert_eq!(
            status,
            StatusCode::CREATED,
            "AP: POST /api/ap/bills must return 201 after period reopen; body={}",
            body
        );
        println!("AP: reopened period correctly returns 201 Created");
    }

    // ── Phase 6: AR invoice → must return 201 ────────────────────────────────
    {
        let ar_router = make_ar_router(ar_pool.clone(), gl_pool.clone());
        let req = ar_create_invoice_request(&tenant_id, ar_customer_id);
        let (status, body) = parse_response(ar_router.oneshot(req).await.unwrap()).await;

        assert_eq!(
            status,
            StatusCode::CREATED,
            "AR: POST /api/ar/invoices must return 201 after period reopen; body={}",
            body
        );
        println!("AR: reopened period correctly returns 201 Created");
    }

    // ── Cleanup ──────────────────────────────────────────────────────────────
    cleanup(&gl_pool, &ap_pool, &ar_pool, &tenant_id).await;
    println!("test_period_close_enforcement: PASSED");
}
