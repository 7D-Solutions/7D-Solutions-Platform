//! E2E: Payments lifecycle — creation, partial allocation, state machine, multi-currency
//!
//! ## Coverage
//! 1. Payment attempt creation → lifecycle transition ATTEMPTING → SUCCEEDED
//! 2. Partial payment allocation: allocate $300 against $500 invoice, verify remaining $200
//! 3. State machine: ATTEMPTING → FAILED_RETRY → ATTEMPTING → SUCCEEDED (retry path)
//! 4. Terminal guard: SUCCEEDED → any transition is rejected (IllegalTransition)
//! 5. Multi-currency: payment allocated in EUR against EUR invoice
//!
//! ## Pattern
//! No Docker, no mocks. Real Payments-postgres (5436) and AR-postgres (5434).
//! Direct function calls to lifecycle transitions and payment allocation.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- payments_lifecycle_e2e --nocapture
//! ```

mod common;

use ar_rs::payment_allocation::{allocate_payment_fifo, AllocatePaymentRequest};
use payments_rs::lifecycle::{
    self, status, transition_to_attempting, transition_to_failed_final, transition_to_failed_retry,
    transition_to_succeeded,
};
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

/// Create a payment attempt in ATTEMPTING status; return its UUID id.
async fn create_payment_attempt(
    pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: &str,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts
             (app_id, payment_id, invoice_id, attempt_no, status,
              idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, 0, 'attempting'::payment_attempt_status, $4, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id)
    .bind(format!("idem-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("create payment attempt failed")
}

/// Get payment attempt status as text.
async fn get_attempt_status(pool: &PgPool, attempt_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM payment_attempts WHERE id = $1",
    )
    .bind(attempt_id)
    .fetch_one(pool)
    .await
    .expect("fetch attempt status failed")
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
    .bind(format!("pay-lifecycle-{}@test.local", Uuid::new_v4()))
    .bind("Payments Lifecycle Test Customer")
    .fetch_one(pool)
    .await
    .expect("create AR customer failed")
}

/// Create AR invoice; return SERIAL id.
async fn create_ar_invoice(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    amount_cents: i64,
    currency: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_invoices
             (app_id, ar_customer_id, status, amount_cents, currency, due_at,
              tilled_invoice_id, updated_at)
         VALUES ($1, $2, 'open', $3, $4, NOW() + INTERVAL '30 days', $5, NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(format!("inv-lifecycle-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("create AR invoice failed")
}

/// Run the payment allocations migration (idempotent).
async fn run_alloc_migration(pool: &PgPool) {
    let sql = include_str!(
        "../../modules/ar/db/migrations/20260217000008_create_payment_allocations.sql"
    );
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("failed to run payment_allocations migration");
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_payments(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn cleanup_ar(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_payment_allocations WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: Lifecycle — ATTEMPTING → SUCCEEDED
// ============================================================================

#[tokio::test]
#[serial]
async fn test_payment_lifecycle_attempting_to_succeeded() {
    let payments_pool = common::get_payments_pool().await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let payment_id = Uuid::new_v4();

    let attempt_id = create_payment_attempt(
        &payments_pool,
        &app_id,
        payment_id,
        "inv-test-001",
    )
    .await;

    assert_eq!(
        get_attempt_status(&payments_pool, attempt_id).await,
        status::ATTEMPTING
    );

    // Transition to SUCCEEDED
    transition_to_succeeded(&payments_pool, attempt_id, "PSP confirmed")
        .await
        .expect("transition to succeeded must work");

    assert_eq!(
        get_attempt_status(&payments_pool, attempt_id).await,
        status::SUCCEEDED
    );

    println!("PASS: ATTEMPTING → SUCCEEDED lifecycle transition");
    cleanup_payments(&payments_pool, &app_id).await;
}

// ============================================================================
// Test 2: Partial payment allocation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_partial_payment_allocation() {
    let ar_pool = common::get_ar_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_alloc_migration(&ar_pool).await;

    let app_id = common::generate_test_tenant();
    let customer_id = create_ar_customer(&ar_pool, &app_id).await;

    // Create $500 invoice
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, 50_000, "USD").await;
    println!("seeded: invoice {} for $500.00", invoice_id);

    // Allocate $300 (partial)
    let result = allocate_payment_fifo(
        &ar_pool,
        &app_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay-partial-{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 30_000,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("partial allocation must succeed");

    assert_eq!(
        result.allocated_amount_cents, 30_000,
        "allocated must be $300.00, got {}",
        result.allocated_amount_cents
    );
    assert_eq!(
        result.unallocated_amount_cents, 0,
        "unallocated must be $0 (payment fully applied), got {}",
        result.unallocated_amount_cents
    );

    // Verify invoice is still open (partial payment doesn't close it)
    let invoice_status: String = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE id = $1",
    )
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await
    .expect("fetch invoice status");

    // Invoice remains open after partial payment
    assert_eq!(
        invoice_status, "open",
        "invoice must remain open after partial payment"
    );

    // Verify remaining balance via DB
    let total_allocated: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents), 0)::BIGINT FROM ar_payment_allocations WHERE invoice_id = $1",
    )
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await
    .expect("query allocations");

    let remaining = 50_000i64 - total_allocated;
    assert_eq!(
        remaining, 20_000,
        "remaining balance must be $200.00, got ${:.2}",
        remaining as f64 / 100.0
    );

    println!(
        "PASS: partial payment — allocated $300 of $500, remaining $200"
    );
    cleanup_ar(&ar_pool, &app_id).await;
}

// ============================================================================
// Test 3: Retry path — ATTEMPTING → FAILED_RETRY → ATTEMPTING → SUCCEEDED
// ============================================================================

#[tokio::test]
#[serial]
async fn test_payment_lifecycle_retry_path() {
    let payments_pool = common::get_payments_pool().await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let payment_id = Uuid::new_v4();

    let attempt_id = create_payment_attempt(
        &payments_pool,
        &app_id,
        payment_id,
        "inv-retry-001",
    )
    .await;

    // ATTEMPTING → FAILED_RETRY
    transition_to_failed_retry(&payments_pool, attempt_id, "PSP timeout")
        .await
        .expect("ATTEMPTING → FAILED_RETRY must work");
    assert_eq!(
        get_attempt_status(&payments_pool, attempt_id).await,
        status::FAILED_RETRY
    );

    // FAILED_RETRY → ATTEMPTING (retry)
    transition_to_attempting(&payments_pool, attempt_id, "retry window")
        .await
        .expect("FAILED_RETRY → ATTEMPTING must work");
    assert_eq!(
        get_attempt_status(&payments_pool, attempt_id).await,
        status::ATTEMPTING
    );

    // ATTEMPTING → SUCCEEDED (retry succeeds)
    transition_to_succeeded(&payments_pool, attempt_id, "PSP confirmed on retry")
        .await
        .expect("ATTEMPTING → SUCCEEDED (after retry) must work");
    assert_eq!(
        get_attempt_status(&payments_pool, attempt_id).await,
        status::SUCCEEDED
    );

    println!("PASS: retry path ATTEMPTING → FAILED_RETRY → ATTEMPTING → SUCCEEDED");
    cleanup_payments(&payments_pool, &app_id).await;
}

// ============================================================================
// Test 4: Terminal guard — SUCCEEDED rejects all transitions
// ============================================================================

#[tokio::test]
#[serial]
async fn test_payment_lifecycle_terminal_guard() {
    let payments_pool = common::get_payments_pool().await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let payment_id = Uuid::new_v4();

    let attempt_id = create_payment_attempt(
        &payments_pool,
        &app_id,
        payment_id,
        "inv-terminal-001",
    )
    .await;

    // Move to SUCCEEDED (terminal)
    transition_to_succeeded(&payments_pool, attempt_id, "PSP confirmed")
        .await
        .expect("transition to succeeded must work");

    // Try SUCCEEDED → FAILED_FINAL (must fail)
    let err = transition_to_failed_final(&payments_pool, attempt_id, "should not work")
        .await
        .expect_err("SUCCEEDED → FAILED_FINAL must be rejected");

    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("Illegal transition") || err_msg.contains("illegal"),
        "error must indicate illegal transition, got: {}",
        err_msg
    );

    // Status must still be SUCCEEDED
    assert_eq!(
        get_attempt_status(&payments_pool, attempt_id).await,
        status::SUCCEEDED,
        "status must remain SUCCEEDED after rejected transition"
    );

    // Try SUCCEEDED → ATTEMPTING (must also fail)
    let err2 = transition_to_attempting(&payments_pool, attempt_id, "should not work")
        .await
        .expect_err("SUCCEEDED → ATTEMPTING must be rejected");

    let err2_msg = format!("{}", err2);
    assert!(
        err2_msg.contains("Illegal transition") || err2_msg.contains("illegal"),
        "error must indicate illegal transition, got: {}",
        err2_msg
    );

    println!("PASS: terminal guard — SUCCEEDED rejects all outgoing transitions");
    cleanup_payments(&payments_pool, &app_id).await;
}

// ============================================================================
// Test 5: Multi-currency payment allocation (EUR)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multi_currency_payment_allocation() {
    let ar_pool = common::get_ar_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_alloc_migration(&ar_pool).await;

    let app_id = common::generate_test_tenant();
    let customer_id = create_ar_customer(&ar_pool, &app_id).await;

    // Create EUR invoice for 200.00 EUR
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, 20_000, "EUR").await;
    println!("seeded: EUR invoice {} for 200.00 EUR", invoice_id);

    // Allocate full amount in EUR
    let result = allocate_payment_fifo(
        &ar_pool,
        &app_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay-eur-{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 20_000,
            currency: "eur".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("EUR allocation must succeed");

    assert_eq!(
        result.allocated_amount_cents, 20_000,
        "allocated must be 200.00 EUR"
    );

    // Verify allocation record in DB
    let db_alloc: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents), 0)::BIGINT FROM ar_payment_allocations WHERE invoice_id = $1",
    )
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await
    .expect("query allocations");

    assert_eq!(
        db_alloc, 20_000,
        "DB allocation must match: 200.00 EUR"
    );

    println!("PASS: multi-currency — EUR payment allocated successfully");
    cleanup_ar(&ar_pool, &app_id).await;
}
