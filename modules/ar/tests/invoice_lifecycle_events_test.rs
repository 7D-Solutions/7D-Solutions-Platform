//! Invoice lifecycle event integration tests — bd-18wm
//!
//! Proves exactly-once emission of ar.invoice_opened and ar.invoice_paid
//! against a real Postgres database. No mocks, no stubs.
//!
//! Invariants verified:
//!   1. create_invoice → exactly one ar.invoice_opened in events_outbox, correct payload
//!   2. handle_payment_succeeded → exactly one ar.invoice_paid in events_outbox, correct payload
//!   3. Replay of both → no duplicate outbox rows (ON CONFLICT DO NOTHING)

mod common;

use ar_rs::consumer_tasks::handle_payment_succeeded;
use ar_rs::models::PaymentSucceededPayload;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

// ── Test 1: invoice_opened emitted on create ──────────────────────────────────

#[tokio::test]
#[serial]
async fn test_invoice_opened_emitted_exactly_once() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Clean outbox entries for this test customer
    sqlx::query("DELETE FROM events_outbox WHERE aggregate_type = 'invoice'")
        .execute(&pool)
        .await
        .ok();

    // Create invoice via HTTP API
    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 7500,
        "currency": "usd"
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let invoice_json = common::body_json(resp).await;
    let invoice_id = invoice_json["id"].as_i64().unwrap() as i32;

    // Assert exactly one invoice_opened outbox row for this invoice
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'ar.invoice_opened' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert_eq!(count, 1, "Expected exactly 1 invoice_opened event");

    // Verify payload fields
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'ar.invoice_opened' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("payload query failed");

    // payload column stores the full EventEnvelope JSON; inner fields are under ["payload"]
    let inner = &payload["payload"];
    assert_eq!(inner["invoice_id"], invoice_id.to_string());
    assert_eq!(inner["customer_id"], customer_id.to_string());
    assert_eq!(inner["app_id"], APP_ID);
    assert_eq!(inner["amount_cents"], 7500);
    assert_eq!(inner["currency"], "usd");
    assert!(inner["paid_at"].is_null(), "paid_at should be null for opened invoice");

    // Verify idempotency key was used (deterministic event_id)
    let event_uuid: uuid::Uuid = sqlx::query_scalar(
        "SELECT event_id FROM events_outbox WHERE event_type = 'ar.invoice_opened' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("event_id query failed");

    let expected_key = format!("ar.events.ar.invoice_opened:{}", invoice_id);
    let expected_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, expected_key.as_bytes());
    assert_eq!(event_uuid, expected_uuid, "event_id must match deterministic UUID v5");

    // Replay: try to insert same event_id again → ON CONFLICT DO NOTHING
    let rows_affected = sqlx::query(
        r#"
        INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload)
        VALUES ($1, 'ar.invoice_opened', 'invoice', $2, '{}')
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_uuid)
    .bind(invoice_id.to_string())
    .execute(&pool)
    .await
    .expect("duplicate insert failed")
    .rows_affected();

    assert_eq!(rows_affected, 0, "Duplicate event must be a no-op");

    // Count still 1
    let count_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'ar.invoice_opened' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count_after, 1, "Count must remain 1 after replay");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

// ── Test 2: invoice_paid emitted on status transition ─────────────────────────

#[tokio::test]
#[serial]
async fn test_invoice_paid_emitted_exactly_once() {
    let pool = common::setup_pool().await;
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed an open invoice directly
    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id,
            status, amount_cents, currency, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', 4200, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(APP_ID)
    .bind(format!("in_test_{}", uuid::Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to seed invoice");

    // Clean outbox for this invoice
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'invoice' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .execute(&pool)
    .await
    .ok();

    // Trigger handle_payment_succeeded
    let payment_payload = PaymentSucceededPayload {
        payment_id: "pay_test_001".to_string(),
        invoice_id: invoice_id.to_string(),
        ar_customer_id: customer_id.to_string(),
        amount_minor: 4200,
        currency: "usd".to_string(),
        processor_payment_id: None,
        payment_method_ref: None,
    };
    handle_payment_succeeded(&pool, &payment_payload)
        .await
        .expect("handle_payment_succeeded failed");

    // Assert exactly one invoice_paid outbox row
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'ar.invoice_paid' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Expected exactly 1 invoice_paid event");

    // Verify payload
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'ar.invoice_paid' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    // payload column stores the full EventEnvelope JSON; inner fields are under ["payload"]
    let inner = &payload["payload"];
    assert_eq!(inner["invoice_id"], invoice_id.to_string());
    assert_eq!(inner["customer_id"], customer_id.to_string());
    assert_eq!(inner["app_id"], APP_ID);
    assert_eq!(inner["amount_cents"], 4200);
    assert!(!inner["paid_at"].is_null(), "paid_at must be set for paid invoice");

    // Verify invoice status in DB
    let status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "paid");

    // Replay: calling handle_payment_succeeded again must not emit a second event
    handle_payment_succeeded(&pool, &payment_payload)
        .await
        .expect("replay of handle_payment_succeeded failed");

    let count_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'ar.invoice_paid' AND aggregate_id = $1",
    )
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count_after, 1, "Replay must not produce a second invoice_paid event");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
