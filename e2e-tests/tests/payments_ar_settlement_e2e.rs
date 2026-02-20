//! E2E: Payments → AR settlement — payment received marks invoice paid (bd-nn3g)
//!
//! Proves the payment settlement chain: a `payments.events.payment.succeeded`
//! event received by the AR module transitions the matching invoice from
//! `open` → `paid`.
//!
//! ## Invariants tested
//! 1. Payment succeeded event → invoice transitions open → paid
//! 2. Idempotency: same event_id processed twice → no error, invoice stays paid
//! 3. Double-payment guard: second payment event for already-paid invoice → no-op
//! 4. Payment attempt record exists in Payments DB when invoice is settled
//!
//! ## Pattern
//! No Docker, no mocks. Real AR-postgres (5434) and Payments-postgres (5436).
//! `process_payment_succeeded` called directly with a fabricated BusMessage —
//! the same code path the live NATS consumer runs.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- payments_ar_settlement_e2e --nocapture
//! ```

mod common;

use ar_rs::consumer_tasks::process_payment_succeeded;
use chrono::NaiveDate;
use event_bus::BusMessage;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn run_ar_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ar/db/migrations")
        .run(pool)
        .await
        .expect("AR migrations failed");
}

async fn run_payments_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/payments/db/migrations")
        .run(pool)
        .await
        .expect("Payments migrations failed");
}

/// Create AR customer; return SERIAL id.
async fn create_ar_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers
             (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("settle-{}@test.example", Uuid::new_v4()))
    .bind("Settlement Test Customer")
    .fetch_one(pool)
    .await
    .expect("create AR customer failed")
}

/// Create AR invoice with status=open; return SERIAL id.
async fn create_ar_invoice(pool: &PgPool, app_id: &str, customer_id: i32, amount: i64) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_invoices
             (app_id, ar_customer_id, status, amount_cents, currency, due_at,
              tilled_invoice_id, updated_at)
         VALUES ($1, $2, 'open', $3, 'USD', $4, $5, NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(amount)
    .bind(NaiveDate::from_ymd_opt(2026, 3, 31).unwrap())
    .bind(format!("inv-settle-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("create AR invoice failed")
}

/// Create a payment attempt record in the Payments DB.
async fn create_payment_attempt(
    pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO payment_attempts
             (app_id, payment_id, invoice_id, attempt_no, status,
              idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, 0, $4::payment_attempt_status, $5, NOW(), NOW())",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .bind(status)
    .bind(format!("idem-{}", Uuid::new_v4()))
    .execute(pool)
    .await
    .expect("create payment attempt failed");
}

/// Build a `payments.events.payment.succeeded` BusMessage.
fn make_payment_succeeded_msg(
    event_id: Uuid,
    tenant_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
    amount_minor: i32,
) -> BusMessage {
    let envelope = serde_json::json!({
        "event_id": event_id.to_string(),
        "occurred_at": "2026-02-20T00:00:00Z",
        "tenant_id": tenant_id,
        "source_module": "payments",
        "source_version": "1.0.0",
        "payload": {
            "payment_id": payment_id.to_string(),
            "invoice_id": invoice_id.to_string(),
            "ar_customer_id": "cust-settle-001",
            "amount_minor": amount_minor,
            "currency": "USD"
        }
    });
    BusMessage {
        subject: "payments.events.payment.succeeded".to_string(),
        payload: serde_json::to_vec(&envelope).unwrap(),
        headers: None,
        reply_to: None,
    }
}

/// Fetch invoice status from AR DB.
async fn get_invoice_status(pool: &PgPool, invoice_id: i32) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(pool)
        .await
        .expect("fetch invoice status failed")
}

/// Count processed events recorded for an event_id.
async fn count_processed_events(pool: &PgPool, event_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM processed_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Count payment attempts by payment_id.
async fn count_payment_attempts(pool: &PgPool, app_id: &str, payment_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(app_id)
    .bind(payment_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup(ar_pool: &PgPool, payments_pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM processed_events WHERE source_module = 'payments'")
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(payments_pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: payment.succeeded → invoice paid
// ============================================================================

/// Invariant: a payment.succeeded event transitions invoice open → paid.
#[tokio::test]
#[serial]
async fn test_payment_succeeded_marks_invoice_paid() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let payment_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    const AMOUNT: i64 = 50_000; // $500.00

    let customer_id = create_ar_customer(&ar_pool, &app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, AMOUNT).await;

    // Record the payment attempt in Payments DB
    create_payment_attempt(&payments_pool, &app_id, payment_id, invoice_id, "succeeded").await;
    assert_eq!(
        count_payment_attempts(&payments_pool, &app_id, payment_id).await,
        1,
        "payment attempt must exist before settlement"
    );

    // Verify invoice starts as open
    assert_eq!(
        get_invoice_status(&ar_pool, invoice_id).await,
        "open",
        "invoice must start as open"
    );

    // Simulate the payments.events.payment.succeeded event arriving at AR
    let msg = make_payment_succeeded_msg(event_id, &app_id, payment_id, invoice_id, AMOUNT as i32);
    process_payment_succeeded(&ar_pool, &msg)
        .await
        .expect("process_payment_succeeded must succeed");

    // Invariant: invoice is now paid
    assert_eq!(
        get_invoice_status(&ar_pool, invoice_id).await,
        "paid",
        "invoice must be paid after payment.succeeded event"
    );

    // Invariant: event is recorded as processed
    assert_eq!(
        count_processed_events(&ar_pool, event_id).await,
        1,
        "payment.succeeded event_id must be recorded in processed_events"
    );

    println!(
        "✅ PASS: payment.succeeded → invoice {} transitioned open → paid (payment_id={})",
        invoice_id, payment_id
    );

    cleanup(&ar_pool, &payments_pool, &app_id).await;
}

// ============================================================================
// Test 2: idempotency — same event_id processed twice → no error
// ============================================================================

/// Invariant: processing the same event_id twice is idempotent.
/// The invoice stays paid; no duplicate processed_events rows; no error.
#[tokio::test]
#[serial]
async fn test_payment_succeeded_idempotent() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let payment_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    const AMOUNT: i64 = 30_000;

    let customer_id = create_ar_customer(&ar_pool, &app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, AMOUNT).await;
    create_payment_attempt(&payments_pool, &app_id, payment_id, invoice_id, "succeeded").await;

    let msg = make_payment_succeeded_msg(event_id, &app_id, payment_id, invoice_id, AMOUNT as i32);

    // First processing
    process_payment_succeeded(&ar_pool, &msg)
        .await
        .expect("first process_payment_succeeded must succeed");
    assert_eq!(get_invoice_status(&ar_pool, invoice_id).await, "paid");

    // Second processing — same event_id
    process_payment_succeeded(&ar_pool, &msg)
        .await
        .expect("second process_payment_succeeded must succeed (idempotent)");

    // Invoice remains paid
    assert_eq!(
        get_invoice_status(&ar_pool, invoice_id).await,
        "paid",
        "invoice must remain paid after duplicate event processing"
    );

    // Only one processed_events row
    assert_eq!(
        count_processed_events(&ar_pool, event_id).await,
        1,
        "processed_events must have exactly one row (ON CONFLICT DO NOTHING)"
    );

    println!(
        "✅ PASS: idempotency — duplicate event_id={} ignored, invoice {} stays paid",
        event_id, invoice_id
    );

    cleanup(&ar_pool, &payments_pool, &app_id).await;
}

// ============================================================================
// Test 3: double-payment guard — already-paid invoice stays paid
// ============================================================================

/// Invariant: a second payment.succeeded event for an already-paid invoice
/// is a safe no-op. The invoice status does not change; no error is raised.
/// This protects against double-settlement when a payment event is replayed.
#[tokio::test]
#[serial]
async fn test_double_payment_guard_already_paid_invoice() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let payment_id_1 = Uuid::new_v4();
    let payment_id_2 = Uuid::new_v4();
    let event_id_1 = Uuid::new_v4();
    let event_id_2 = Uuid::new_v4();
    const AMOUNT: i64 = 20_000;

    let customer_id = create_ar_customer(&ar_pool, &app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, AMOUNT).await;

    // First payment settles the invoice
    create_payment_attempt(&payments_pool, &app_id, payment_id_1, invoice_id, "succeeded").await;
    let msg1 =
        make_payment_succeeded_msg(event_id_1, &app_id, payment_id_1, invoice_id, AMOUNT as i32);
    process_payment_succeeded(&ar_pool, &msg1)
        .await
        .expect("first payment must succeed");
    assert_eq!(get_invoice_status(&ar_pool, invoice_id).await, "paid");

    // Second payment event for the same invoice (different event_id, different payment_id)
    create_payment_attempt(&payments_pool, &app_id, payment_id_2, invoice_id, "succeeded").await;
    let msg2 =
        make_payment_succeeded_msg(event_id_2, &app_id, payment_id_2, invoice_id, AMOUNT as i32);
    process_payment_succeeded(&ar_pool, &msg2)
        .await
        .expect("second payment.succeeded must not return an error");

    // Invoice is still paid (the UPDATE is guarded by `status != 'paid'`)
    assert_eq!(
        get_invoice_status(&ar_pool, invoice_id).await,
        "paid",
        "invoice must remain paid; double-payment must be a silent no-op"
    );

    // Both events are recorded as processed (each event_id is unique)
    assert_eq!(
        count_processed_events(&ar_pool, event_id_1).await,
        1,
        "first event must be in processed_events"
    );
    assert_eq!(
        count_processed_events(&ar_pool, event_id_2).await,
        1,
        "second event must be in processed_events"
    );

    // Two payment attempt records exist (one per payment_id)
    assert_eq!(
        count_payment_attempts(&payments_pool, &app_id, payment_id_1).await,
        1
    );
    assert_eq!(
        count_payment_attempts(&payments_pool, &app_id, payment_id_2).await,
        1
    );

    println!(
        "✅ PASS: double-payment guard — invoice {} stays paid after second payment event",
        invoice_id
    );

    cleanup(&ar_pool, &payments_pool, &app_id).await;
}
