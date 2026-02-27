//! Payment loop E2E integration tests — bd-1s7f
//!
//! Proves the full money path against real services (real Postgres, real AR
//! HTTP router). No mocking, no stubs.
//!
//! Flow under test:
//!   customer → invoice → webhook(invoice.payment_succeeded) → invoice.status = paid
//!
//! Idempotency invariant:
//!   Replaying the same Tilled webhook event MUST NOT produce a second
//!   ar_webhooks row. The invoice status MUST remain unchanged.
//!
//! Run with:
//!   DATABASE_URL_AR=... cargo test -p ar-rs --test payment_loop_e2e

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, Mac};
use serial_test::serial;
use sha2::Sha256;
use tower::ServiceExt;

const APP_ID: &str = "00000000-0000-0000-0000-000000000001";
const WEBHOOK_SECRET: &str = "whsec_loop_test_secret";

/// Set the webhook secret env vars so the AR router accepts our test payloads.
fn configure_webhook_secret() {
    std::env::set_var("TILLED_WEBHOOK_SECRET", WEBHOOK_SECRET);
    std::env::set_var("TILLED_WEBHOOK_SECRET_TRASHTECH", WEBHOOK_SECRET);
}

/// Compute a Tilled-format HMAC-SHA256 signature: "t=<ts>,v1=<hex>".
fn tilled_signature(payload: &str, ts: i64) -> String {
    let signed = format!("{}.{}", ts, payload);
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(signed.as_bytes());
    let hex = hex::encode(mac.finalize().into_bytes());
    format!("t={},v1={}", ts, hex)
}

/// Deliver a webhook payload to the AR router and return the HTTP status.
async fn deliver_webhook(
    app: axum::Router,
    payload_str: &str,
    ts: i64,
) -> StatusCode {
    let sig = tilled_signature(payload_str, ts);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .header("tilled-signature", sig)
                .header("x-tilled-account", APP_ID)
                .body(Body::from(payload_str.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

// ── Test 1: Full payment loop ─────────────────────────────────────────────────
/// End-to-end: create customer → create invoice → deliver payment webhook →
/// assert invoice.status = "paid".
#[tokio::test]
#[serial]
async fn test_payment_loop_invoice_marked_paid() {
    configure_webhook_secret();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create customer
    let email = common::unique_email();
    let create_customer = serde_json::json!({
        "email": email,
        "name": "Loop Test Customer",
        "external_customer_id": common::unique_external_id()
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_customer).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "create customer");
    let customer = common::body_json(resp).await;
    let customer_id = customer["id"].as_i64().unwrap() as i32;

    // Step 2: Create invoice (AR generates a tilled_invoice_id)
    let create_invoice = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 5000,
        "currency": "usd",
        "status": "open",
        "metadata": {"source": "payment_loop_e2e"}
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_invoice).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "create invoice");
    let invoice = common::body_json(resp).await;
    let invoice_id = invoice["id"].as_i64().unwrap() as i32;
    let tilled_invoice_id = invoice["tilled_invoice_id"].as_str().unwrap().to_owned();

    assert!(!tilled_invoice_id.is_empty(), "tilled_invoice_id must be set by AR");

    // Step 3: Deliver invoice.payment_succeeded webhook referencing the tilled_invoice_id
    let event_id = format!("evt_loop_{}", uuid::Uuid::new_v4());
    let ts = chrono::Utc::now().timestamp();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "invoice.payment_succeeded",
        "data": {
            "id": tilled_invoice_id,
            "status": "paid",
            "amount": 5000,
            "currency": "usd"
        },
        "created_at": ts,
        "livemode": false
    });
    let payload_str = serde_json::to_string(&payload).unwrap();

    let status = deliver_webhook(app.clone(), &payload_str, ts).await;
    assert_eq!(status, StatusCode::OK, "webhook delivery must return 200");

    // Step 4: Verify invoice status = paid
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/ar/invoices/{}", invoice_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "get invoice after webhook");
    let inv = common::body_json(resp).await;
    assert_eq!(
        inv["status"].as_str().unwrap(),
        "paid",
        "invoice must be marked paid after payment webhook"
    );

    // Cleanup
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

// ── Test 2: Webhook replay idempotency ────────────────────────────────────────
/// Delivering the same Tilled event_id twice MUST NOT create a second
/// ar_webhooks row and MUST NOT corrupt the invoice status.
#[tokio::test]
#[serial]
async fn test_payment_loop_webhook_replay_is_idempotent() {
    configure_webhook_secret();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Create customer + invoice
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let create_invoice = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 2500,
        "currency": "usd",
        "status": "open",
        "metadata": {"source": "idempotency_e2e"}
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_invoice).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "create invoice");
    let invoice = common::body_json(resp).await;
    let invoice_id = invoice["id"].as_i64().unwrap() as i32;
    let tilled_invoice_id = invoice["tilled_invoice_id"].as_str().unwrap().to_owned();

    // Build event payload (fixed event_id — will be replayed)
    let event_id = format!("evt_idem_{}", uuid::Uuid::new_v4());
    let base_ts = chrono::Utc::now().timestamp();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "invoice.payment_succeeded",
        "data": {
            "id": tilled_invoice_id,
            "status": "paid",
            "amount": 2500,
            "currency": "usd"
        },
        "created_at": base_ts,
        "livemode": false
    });
    let payload_str = serde_json::to_string(&payload).unwrap();

    // Delivery 1
    let ts1 = chrono::Utc::now().timestamp();
    let s1 = deliver_webhook(app.clone(), &payload_str, ts1).await;
    assert_eq!(s1, StatusCode::OK, "first delivery must succeed");

    // Delivery 2 — replay of the same event_id (new timestamp for freshness window)
    let ts2 = chrono::Utc::now().timestamp();
    let s2 = deliver_webhook(app.clone(), &payload_str, ts2).await;
    assert_eq!(s2, StatusCode::OK, "replay must return 200 (idempotent, not error)");

    // Verify: exactly ONE ar_webhooks row for this event_id
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_webhooks WHERE event_id = $1",
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .expect("count query");

    assert_eq!(count, 1, "exactly one webhook record must exist (replay must not double-post)");

    // Verify invoice status is still paid (not corrupted by replay)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/ar/invoices/{}", invoice_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let inv = common::body_json(resp).await;
    assert_eq!(
        inv["status"].as_str().unwrap(),
        "paid",
        "invoice status must remain paid after replay"
    );

    // Cleanup
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

// ── Test 3: Double-replay does not increase webhook count ─────────────────────
/// Replay three times — row count remains 1.
#[tokio::test]
#[serial]
async fn test_payment_loop_multiple_replays_single_record() {
    configure_webhook_secret();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let create_invoice = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 1000,
        "currency": "usd",
        "status": "open"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_invoice).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let invoice = common::body_json(resp).await;
    let tilled_invoice_id = invoice["tilled_invoice_id"].as_str().unwrap().to_owned();

    let event_id = format!("evt_multi_{}", uuid::Uuid::new_v4());
    let base_ts = chrono::Utc::now().timestamp();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "invoice.payment_succeeded",
        "data": {"id": tilled_invoice_id, "status": "paid"},
        "created_at": base_ts,
        "livemode": false
    });
    let payload_str = serde_json::to_string(&payload).unwrap();

    // Deliver 3 times
    for i in 0..3_u64 {
        // Stagger timestamps slightly to stay inside the freshness window
        let ts = chrono::Utc::now().timestamp() + i as i64;
        let s = deliver_webhook(app.clone(), &payload_str, ts).await;
        assert_eq!(s, StatusCode::OK, "delivery {} must return 200", i + 1);
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_webhooks WHERE event_id = $1",
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .expect("count query");

    assert_eq!(
        count, 1,
        "three deliveries of the same event_id must produce exactly one record"
    );

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
