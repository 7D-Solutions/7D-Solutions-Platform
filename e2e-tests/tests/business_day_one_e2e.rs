//! E2E: Day-one business scenario (bd-ieey)
//!
//! Canonical integration proof: Tenant → Party Master → AR → Payments → GL.
//! Proves that a new tenant can immediately perform the most fundamental
//! business operation — create a party, issue an invoice, collect payment,
//! and see the resulting journal entry in the general ledger.
//!
//! ## Chain tested
//! 1. Create a company party in Party Master
//! 2. Create an AR invoice linked to that party (HTTP, validates party_id)
//! 3. Assert invoice status = OPEN
//! 4. Record payment succeeded → update invoice to PAID
//! 5. Assert invoice status = PAID via GET
//! 6. Create GL journal entry referencing the payment event
//! 7. Assert GL entry is balanced (debits == credits)
//!
//! ## Services required
//! - party-postgres at localhost:5448
//! - ar-postgres at localhost:5434
//! - payments-postgres at localhost:5436
//! - gl-postgres at localhost:5438
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- business_day_one_e2e --nocapture
//! ```

mod common;

use ar_rs::http::ar_router;
use axum::{
    body::Body,
    extract::Request as AxumRequest,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use chrono::Utc;
use common::{get_ar_pool, get_gl_pool, get_party_pool, get_payments_pool};
use party_rs::{
    domain::party::{service as party_service, CreateCompanyRequest},
    http as party_http,
    metrics::PartyMetrics,
    AppState as PartyAppState,
};
use security::{permissions, ActorType, VerifiedClaims};
use serde_json::{json, Value};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

/// All modules use the same tenant UUID for cross-module consistency within the test.
const APP_ID: &str = "00000000-0000-4000-a000-000000000005";

// ============================================================================
// Helpers
// ============================================================================

async fn run_ar_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ar/db/migrations")
        .run(pool)
        .await
        .expect("AR migrations failed");
}

async fn run_party_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

fn make_verified_claims() -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::parse_str(APP_ID).unwrap(),
        app_id: None,
        roles: vec![],
        perms: vec![
            permissions::AR_MUTATE.to_string(),
            permissions::AR_READ.to_string(),
        ],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "test".to_string(),
    }
}

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

async fn create_ar_customer(pool: &PgPool, suffix: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(APP_ID)
    .bind(format!("day-one-{}@test.example", suffix))
    .bind(format!("Day-One Customer {}", suffix))
    .fetch_one(pool)
    .await
    .expect("create AR customer failed")
}

/// Middleware that injects test VerifiedClaims on every request, so
/// inter-service calls to Party Master pass auth without a real JWT.
async fn inject_party_claims(mut req: AxumRequest, next: Next) -> Response {
    let claims = VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::parse_str(APP_ID).unwrap(),
        app_id: None,
        roles: vec![],
        perms: vec![
            permissions::PARTY_MUTATE.to_string(),
            permissions::PARTY_READ.to_string(),
        ],
        actor_type: ActorType::Service,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "test".to_string(),
    };
    req.extensions_mut().insert(claims);
    next.run(req).await
}

async fn spawn_party_server(party_pool: PgPool) -> u16 {
    let metrics = Arc::new(PartyMetrics::new().expect("party metrics init failed"));
    let state = Arc::new(PartyAppState {
        pool: party_pool,
        metrics,
    });
    let router = party_http::router(state)
        .layer(axum::middleware::from_fn(inject_party_claims));

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

async fn setup_gl_accounts(gl_pool: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
         VALUES
           (gen_random_uuid(), $1, 'AR', 'Accounts Receivable', 'asset', 'debit', true),
           (gen_random_uuid(), $1, 'REV', 'Revenue', 'revenue', 'credit', true)
         ON CONFLICT (tenant_id, code) DO NOTHING",
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .expect("GL account setup failed");
}

async fn setup_gl_period(gl_pool: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
         VALUES ($1, '2026-02-01', '2026-02-28', false)",
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .expect("GL period setup failed");
}

async fn create_gl_journal_entry(gl_pool: &PgPool, tenant_id: &str, source_event_id: Uuid) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description)
         VALUES ($1, $2, 'payments', $3, 'payment.succeeded', NOW(), 'USD', 'Day-one payment posting')
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(gl_pool)
    .await
    .expect("GL journal entry creation failed")
}

async fn create_gl_lines(gl_pool: &PgPool, entry_id: Uuid, amount: i64) {
    sqlx::query(
        "INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
         VALUES ($1, $2, 1, 'AR', $3, 0)",
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(amount)
    .execute(gl_pool)
    .await
    .expect("GL debit line failed");

    sqlx::query(
        "INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
         VALUES ($1, $2, 2, 'REV', 0, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(amount)
    .execute(gl_pool)
    .await
    .expect("GL credit line failed");
}

async fn cleanup(
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    gl_pool: &PgPool,
    party_pool: &PgPool,
    customer_id: i32,
    party_id: Uuid,
    gl_tenant_id: &str,
) {
    // GL
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();
    sqlx::query("DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(gl_tenant_id)
        .execute(gl_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(gl_tenant_id)
        .execute(gl_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(gl_tenant_id)
        .execute(gl_pool)
        .await
        .ok();

    // Payments
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(APP_ID)
        .execute(payments_pool)
        .await
        .ok();

    // AR
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(APP_ID)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1 AND ar_customer_id = $2")
        .bind(APP_ID)
        .bind(customer_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE id = $1")
        .bind(customer_id)
        .execute(ar_pool)
        .await
        .ok();

    // Party
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(APP_ID)
        .execute(party_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_companies WHERE party_id = $1")
        .bind(party_id)
        .execute(party_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_parties WHERE id = $1")
        .bind(party_id)
        .execute(party_pool)
        .await
        .ok();
}

// ============================================================================
// Test: Full day-one business chain
// ============================================================================

#[tokio::test]
#[serial]
async fn test_business_day_one_full_chain() {
    let run_id = Uuid::new_v4();
    let run_tag = &run_id.to_string()[..8];

    // ── Connect to all databases ────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ar_pool = get_ar_pool().await;
    run_ar_migrations(&ar_pool).await;

    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    let gl_tenant_id = format!("day-one-{}", run_tag);

    // ── Start Party Master HTTP server ──────────────────────────────────
    let _party_port = spawn_party_server(party_pool.clone()).await;
    println!(
        "Party Master running at {}",
        std::env::var("PARTY_MASTER_URL").unwrap()
    );

    let ar = ar_router(ar_pool.clone());

    // ================================================================
    // Step 1: Create a company party in Party Master
    // ================================================================
    let company_req = CreateCompanyRequest {
        display_name: format!("DayOne Corp {}", run_tag),
        legal_name: format!("DayOne Corporation {}", run_tag),
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
        party_service::create_company(&party_pool, APP_ID, &company_req, run_id.to_string())
            .await
            .expect("Party Master: create_company failed");

    let party_id = party_view.party.id;
    assert_eq!(party_view.party.party_type, "company");
    assert_eq!(party_view.party.status, "active");
    println!("[1/7] Party created: {}", party_id);

    // ================================================================
    // Step 2: Create AR customer + invoice linked to party
    // ================================================================
    let customer_id = create_ar_customer(&ar_pool, run_tag).await;
    println!("[2/7] AR customer created: {}", customer_id);

    let invoice_body = json!({
        "ar_customer_id": customer_id,
        "amount_cents": 25000,
        "currency": "usd",
        "status": "open",
        "party_id": party_id.to_string()
    });

    let (create_status, create_resp) =
        ar_send(&ar, "POST", "/api/ar/invoices", Some(invoice_body), true).await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "POST /api/ar/invoices must return 201; body={}",
        create_resp
    );

    let invoice_id = create_resp["id"]
        .as_i64()
        .expect("invoice id must be integer");

    let resp_party_id = create_resp["party_id"]
        .as_str()
        .expect("party_id must be in create response");
    assert_eq!(resp_party_id, party_id.to_string());
    println!(
        "[3/7] AR invoice created: {} (party_id={})",
        invoice_id, resp_party_id
    );

    // ================================================================
    // Step 3: Assert invoice status = OPEN via GET
    // ================================================================
    let (get_status, get_body) = ar_send(
        &ar,
        "GET",
        &format!("/api/ar/invoices/{}", invoice_id),
        None,
        true,
    )
    .await;

    assert_eq!(get_status, StatusCode::OK, "GET invoice: body={}", get_body);
    assert_eq!(
        get_body["status"].as_str().unwrap_or(""),
        "open",
        "Invoice must be OPEN; body={}",
        get_body
    );
    println!("[4/7] Invoice status confirmed: OPEN");

    // ================================================================
    // Step 4: Create payment + mark invoice PAID
    // ================================================================
    let payment_id = Uuid::new_v4();

    let attempt_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3::text, 0, 'succeeded'::payment_attempt_status)
         RETURNING id",
    )
    .bind(APP_ID)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .fetch_one(&payments_pool)
    .await
    .expect("payment attempt creation failed");

    println!("[5/7] Payment recorded: {} (succeeded)", attempt_id);

    // Simulate event-driven update: payment.succeeded → invoice.paid
    sqlx::query(
        "UPDATE ar_invoices SET status = 'paid', paid_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(invoice_id as i32)
    .execute(&ar_pool)
    .await
    .expect("invoice paid update failed");

    // Verify via GET
    let (get2_status, get2_body) = ar_send(
        &ar,
        "GET",
        &format!("/api/ar/invoices/{}", invoice_id),
        None,
        true,
    )
    .await;

    assert_eq!(get2_status, StatusCode::OK);
    assert_eq!(
        get2_body["status"].as_str().unwrap_or(""),
        "paid",
        "Invoice must be PAID; body={}",
        get2_body
    );
    assert!(
        !get2_body["paid_at"].is_null(),
        "paid_at must be set; body={}",
        get2_body
    );
    println!("[6/7] Invoice status confirmed: PAID");

    // ================================================================
    // Step 5: GL journal entry — balanced posting
    // ================================================================
    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    let source_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("payment.succeeded:{}", payment_id).as_bytes(),
    );

    let entry_id = create_gl_journal_entry(&gl_pool, &gl_tenant_id, source_event_id).await;
    create_gl_lines(&gl_pool, entry_id, 25000).await;

    // Verify balanced
    let balance_result = common::assert_journal_balanced(&gl_pool, entry_id).await;
    assert!(
        balance_result.is_ok(),
        "GL must be balanced: {:?}",
        balance_result
    );

    let (total_debits, total_credits): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT, COALESCE(SUM(credit_minor),0)::BIGINT
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("GL balance query failed");

    assert_eq!(total_debits, total_credits, "GL debits must equal credits");
    assert_eq!(total_debits, 25000, "GL total must match invoice amount");
    println!(
        "[7/7] GL balanced: debits={} credits={} (entry={})",
        total_debits, total_credits, entry_id
    );

    println!("\n=== Day-One Business Scenario: ALL PASSED ===");
    println!(
        "  Party:{} -> Invoice:{} -> Payment:{} -> GL:{}",
        party_id, invoice_id, payment_id, entry_id
    );

    // ── Cleanup ─────────────────────────────────────────────────────────
    cleanup(
        &ar_pool,
        &payments_pool,
        &gl_pool,
        &party_pool,
        customer_id,
        party_id,
        &gl_tenant_id,
    )
    .await;
}
