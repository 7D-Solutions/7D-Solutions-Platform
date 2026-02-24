//! E2E: AP payment run — vendor bill → approved → payment run → paid (bd-2kdz)
//!
//! Proves the AP payment run lifecycle end-to-end with a Party Master–linked vendor:
//!
//! 1. Create a company party in Party Master.
//! 2. Create an AP vendor via HTTP with that party_id.
//! 3. Create a vendor bill against the vendor (domain layer).
//! 4. Approve the bill (domain layer).
//! 5. Create a payment run (domain layer) — bill is selected automatically.
//! 6. Execute the payment run (domain layer).
//! 7. Assert: bill status = "paid", run status = "completed",
//!    total_minor and bill count are correct.
//! 8. Assert: party_id round-trips correctly on the vendor GET response.
//!
//! ## Services required
//! - ap-postgres at localhost:5443
//! - party-postgres at localhost:5448
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- ap_payment_run_e2e --nocapture
//! ```

mod common;

use ap::{
    domain::{
        bills::{
            approve::approve_bill, service::create_bill, ApproveBillRequest,
            CreateBillLineRequest, CreateBillRequest,
        },
        payment_runs::{
            builder::create_payment_run, execute::execute_payment_run, CreatePaymentRunRequest,
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

const TENANT_ID: &str = "ap-payment-run-e2e-tenant";

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

/// Build an in-process AP Axum router wired to the given pool.
fn make_ap_router(pool: PgPool) -> Router {
    let metrics = Arc::new(ApMetrics::new().expect("AP metrics init failed"));
    let state = Arc::new(AppState { pool, metrics });

    let ap_mutations = Router::new()
        .route("/api/ap/vendors", post(http::vendors::create_vendor))
        .route(
            "/api/ap/vendors/{vendor_id}",
            put(http::vendors::update_vendor),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::AP_MUTATE]))
        .with_state(state.clone());

    Router::new()
        .route("/api/ap/vendors/{vendor_id}", get(http::vendors::get_vendor))
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

/// Send a request through the in-process AP router.
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
/// Sets PARTY_MASTER_URL env var so AP party validation picks it up.
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
    let port = listener.local_addr().unwrap().port();

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

/// Clean up all AP data created for TENANT_ID.
async fn cleanup_ap(pool: &PgPool) {
    for q in [
        "DELETE FROM payment_run_executions WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM ap_allocations WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_id IN \
         (SELECT run_id::TEXT FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_run_items WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_runs WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
         AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' AND aggregate_id IN \
         (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TENANT_ID).execute(pool).await.ok();
    }
}

/// Clean up party rows.
async fn cleanup_party(pool: &PgPool, party_id: Uuid) {
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(TENANT_ID)
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
// Test: Full lifecycle — party → vendor → bill → approve → run → execute
// ============================================================================

/// Full AP payment run lifecycle with a Party Master–linked vendor.
///
/// Steps:
/// 1. Create a company party in Party Master (domain call).
/// 2. POST /api/ap/vendors with that party_id (in-process HTTP).
/// 3. Create a vendor bill via domain layer.
/// 4. Approve the bill via domain layer.
/// 5. Create a payment run — bill is auto-selected.
/// 6. Execute the payment run.
/// 7. Assert bill is "paid", run is "completed" with correct total_minor and bill count.
/// 8. Assert GET /api/ap/vendors/:id returns the original party_id.
#[tokio::test]
#[serial]
async fn test_ap_payment_run_full_lifecycle_with_party_id() {
    // ── Connect ──────────────────────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = get_ap_pool().await;
    run_ap_migrations(&ap_pool).await;
    cleanup_ap(&ap_pool).await;

    // ── Start in-process Party Master server ─────────────────────────────────
    let _party_port = spawn_party_server(party_pool.clone()).await;
    println!(
        "Party Master at {}",
        std::env::var("PARTY_MASTER_URL").unwrap()
    );

    // ── Step 1: Create a company party ───────────────────────────────────────
    let run_id = Uuid::new_v4();
    let company_req = CreateCompanyRequest {
        display_name: format!("AP Run Corp {}", &run_id.to_string()[..8]),
        legal_name: format!("AP Run Corporation {}", &run_id.to_string()[..8]),
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
        party_service::create_company(&party_pool, TENANT_ID, &company_req, run_id.to_string())
            .await
            .expect("create_company failed");
    let party_id = party_view.party.id;
    println!("Created party: {}", party_id);

    // ── Step 2: POST /api/ap/vendors with party_id ───────────────────────────
    let ap = make_ap_router(ap_pool.clone());

    let (create_status, create_resp) = ap_send(
        &ap,
        "POST",
        "/api/ap/vendors",
        Some(json!({
            "name": format!("Run Vendor {}", &run_id.to_string()[..8]),
            "currency": "USD",
            "payment_terms_days": 30,
            "payment_method": "ach",
            "party_id": party_id.to_string()
        })),
        true,
        TENANT_ID,
    )
    .await;

    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "POST /api/ap/vendors must return 201; body={}",
        create_resp
    );
    let vendor_id_str = create_resp["vendor_id"]
        .as_str()
        .expect("vendor_id in create response");
    let vendor_id = Uuid::parse_str(vendor_id_str).expect("vendor_id must be a UUID");

    let returned_party_id = create_resp["party_id"]
        .as_str()
        .expect("party_id in create response");
    assert_eq!(
        returned_party_id,
        party_id.to_string(),
        "create response party_id must match"
    );
    println!("Created vendor: {} with party_id: {}", vendor_id, party_id);

    // ── Step 3: Create a vendor bill ─────────────────────────────────────────
    let amount_minor: i64 = 45_000; // $450.00
    let bill_with_lines = create_bill(
        &ap_pool,
        TENANT_ID,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: format!("INV-RUN-{}", &run_id.to_string()[..8]),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: Some(
                chrono::DateTime::parse_from_rfc3339("2026-03-15T00:00:00Z")
                    .expect("parse due_date")
                    .with_timezone(&Utc),
            ),
            tax_minor: None,
            entered_by: "ap-clerk-e2e".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("E2E services".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: amount_minor,
                gl_account_code: Some("6100".to_string()),
                po_line_id: None,
            }],
        },
        format!("corr-create-{}", run_id),
    )
    .await
    .expect("create_bill failed");

    let bill_id = bill_with_lines.bill.bill_id;
    assert_eq!(bill_with_lines.bill.status, "open");
    assert_eq!(bill_with_lines.bill.total_minor, amount_minor);
    println!("Created bill: {} for {} minor units", bill_id, amount_minor);

    // ── Step 4: Approve the bill ─────────────────────────────────────────────
    approve_bill(
        &ap_pool,
        &ZeroTaxProvider,
        TENANT_ID,
        bill_id,
        &ApproveBillRequest {
            approved_by: "controller-e2e".to_string(),
            override_reason: Some("e2e-unmatched-override".to_string()),
        },
        format!("corr-approve-{}", run_id),
    )
    .await
    .expect("approve_bill failed");

    // Verify approved status in DB
    let (bill_status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill_id)
            .bind(TENANT_ID)
            .fetch_one(&ap_pool)
            .await
            .expect("fetch bill status after approve");
    assert_eq!(bill_status, "approved", "bill must be approved before run");
    println!("Bill {} approved", bill_id);

    // ── Step 5: Create a payment run ─────────────────────────────────────────
    let payment_run_id = Uuid::new_v4();
    let run_result = create_payment_run(
        &ap_pool,
        TENANT_ID,
        &CreatePaymentRunRequest {
            run_id: payment_run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "treasurer-e2e".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: Some(format!("corr-run-{}", run_id)),
        },
    )
    .await
    .expect("create_payment_run failed");

    assert_eq!(run_result.run.status, "pending");
    assert_eq!(
        run_result.run.total_minor, amount_minor,
        "payment run total must equal bill amount"
    );
    assert_eq!(run_result.items.len(), 1, "one vendor item in the run");
    let run_item = &run_result.items[0];
    assert_eq!(run_item.vendor_id, vendor_id);
    assert!(
        run_item.bill_ids.contains(&bill_id),
        "run item must include our bill"
    );
    println!(
        "Payment run {} created: total={}, items={}",
        payment_run_id,
        run_result.run.total_minor,
        run_result.items.len()
    );

    // ── Step 6: Execute the payment run ──────────────────────────────────────
    let exec_result = execute_payment_run(&ap_pool, TENANT_ID, payment_run_id)
        .await
        .expect("execute_payment_run failed");

    assert_eq!(
        exec_result.run.status, "completed",
        "payment run must be completed after execution"
    );
    assert!(
        exec_result.run.executed_at.is_some(),
        "executed_at must be set"
    );
    assert_eq!(
        exec_result.executions.len(),
        1,
        "one execution record (one vendor)"
    );
    assert_eq!(exec_result.executions[0].status, "success");
    assert_eq!(exec_result.executions[0].vendor_id, vendor_id);
    println!(
        "Payment run {} executed: status=completed, executions={}",
        payment_run_id,
        exec_result.executions.len()
    );

    // ── Step 7: Verify bill is paid ───────────────────────────────────────────
    let (final_bill_status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill_id)
            .bind(TENANT_ID)
            .fetch_one(&ap_pool)
            .await
            .expect("fetch final bill status");
    assert_eq!(
        final_bill_status, "paid",
        "bill must be paid after payment run execution"
    );
    println!("Bill {} is now paid", bill_id);

    // Verify allocation created for the bill
    let (alloc_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ap_allocations WHERE bill_id = $1 AND payment_run_id = $2 AND tenant_id = $3",
    )
    .bind(bill_id)
    .bind(payment_run_id)
    .bind(TENANT_ID)
    .fetch_one(&ap_pool)
    .await
    .expect("fetch allocation count");
    assert_eq!(alloc_count, 1, "exactly one allocation per bill per run");

    // Verify ap.payment_executed event in outbox
    let (event_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.payment_executed' AND aggregate_id = $1",
    )
    .bind(payment_run_id.to_string())
    .fetch_one(&ap_pool)
    .await
    .expect("fetch event count");
    assert_eq!(event_count, 1, "ap.payment_executed event must be in outbox");

    // ── Step 8: Verify party_id round-trips on vendor GET ────────────────────
    let (get_status, get_resp) = ap_send(
        &ap,
        "GET",
        &format!("/api/ap/vendors/{}", vendor_id),
        None,
        false,
        TENANT_ID,
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
        "GET vendor party_id must round-trip correctly"
    );
    println!("party_id round-trip verified: {}", get_party_id);

    // Verify party_id also in DB
    let db_party_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT party_id FROM vendors WHERE vendor_id = $1 AND tenant_id = $2",
    )
    .bind(vendor_id)
    .bind(TENANT_ID)
    .fetch_one(&ap_pool)
    .await
    .expect("DB query for party_id");
    assert_eq!(
        db_party_id,
        Some(party_id),
        "party_id must be persisted in vendors table"
    );

    // ── Cleanup ───────────────────────────────────────────────────────────────
    cleanup_ap(&ap_pool).await;
    cleanup_party(&party_pool, party_id).await;

    println!(
        "✅ ap_payment_run_e2e PASS: party→vendor→bill→approve→run→execute lifecycle verified"
    );
}

// ============================================================================
// Test: party_id on bill is discoverable via vendor relationship
// ============================================================================

/// Verifies that the party_id stored on the vendor is accessible when
/// looking up the bill's vendor. The bill itself references vendor_id;
/// the vendor carries party_id. This confirms the party_id is
/// transitively queryable from the bill.
#[tokio::test]
#[serial]
async fn test_bill_vendor_party_id_queryable() {
    // ── Connect ──────────────────────────────────────────────────────────────
    let party_pool = get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = get_ap_pool().await;
    run_ap_migrations(&ap_pool).await;
    cleanup_ap(&ap_pool).await;

    let _party_port = spawn_party_server(party_pool.clone()).await;

    let ap = make_ap_router(ap_pool.clone());
    let run_id = Uuid::new_v4();

    // Create party
    let party_view = party_service::create_company(
        &party_pool,
        TENANT_ID,
        &CreateCompanyRequest {
            display_name: format!("Bill Party Corp {}", &run_id.to_string()[..8]),
            legal_name: format!("Bill Party Corporation {}", &run_id.to_string()[..8]),
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
        },
        run_id.to_string(),
    )
    .await
    .expect("create_company failed");
    let party_id = party_view.party.id;

    // Create vendor with party_id
    let (create_status, create_resp) = ap_send(
        &ap,
        "POST",
        "/api/ap/vendors",
        Some(json!({
            "name": format!("Bill Party Vendor {}", &run_id.to_string()[..8]),
            "currency": "USD",
            "payment_terms_days": 30,
            "party_id": party_id.to_string()
        })),
        true,
        TENANT_ID,
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED, "body={}", create_resp);
    let vendor_id = Uuid::parse_str(
        create_resp["vendor_id"]
            .as_str()
            .expect("vendor_id in response"),
    )
    .expect("vendor_id UUID");

    // Create a bill against the vendor
    let bill = create_bill(
        &ap_pool,
        TENANT_ID,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: format!("INV-PARTY-{}", &run_id.to_string()[..8]),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: Some(
                chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            tax_minor: None,
            entered_by: "ap-clerk-e2e".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Party bill item".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: 25_000,
                gl_account_code: Some("6200".to_string()),
                po_line_id: None,
            }],
        },
        format!("corr-party-bill-{}", run_id),
    )
    .await
    .expect("create_bill");
    let bill_id = bill.bill.bill_id;

    // Query: get vendor_id from bill, then party_id from vendor
    let (bill_vendor_id,): (Uuid,) =
        sqlx::query_as("SELECT vendor_id FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill_id)
            .bind(TENANT_ID)
            .fetch_one(&ap_pool)
            .await
            .expect("fetch bill vendor_id");
    assert_eq!(bill_vendor_id, vendor_id);

    let db_party_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT party_id FROM vendors WHERE vendor_id = $1 AND tenant_id = $2",
    )
    .bind(bill_vendor_id)
    .bind(TENANT_ID)
    .fetch_one(&ap_pool)
    .await
    .expect("fetch vendor party_id");

    assert_eq!(
        db_party_id,
        Some(party_id),
        "party_id must be queryable transitively via bill.vendor_id → vendor.party_id"
    );
    println!(
        "✅ party_id transitively queryable from bill {}: party_id={}",
        bill_id, party_id
    );

    // Cleanup
    cleanup_ap(&ap_pool).await;
    cleanup_party(&party_pool, party_id).await;
}
