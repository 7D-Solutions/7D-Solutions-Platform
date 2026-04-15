//! Payments Health Canary E2E (bd-b6z7h)
//!
//! Detects pool starvation or DB hangs in the Payments + AR path within 60s.
//!
//! Full happy-path:
//!   1. Create AR customer
//!   2. Add payment method (card, marked as default)
//!   3. Create payment attempt (ATTEMPTING → SUCCEEDED)  ← "charge"
//!   4. Assert charge attempt status = succeeded
//!   5. Create AR charge record (status = succeeded)     ← domain-level charge record
//!   6. Create AR refund against the charge (status = succeeded)
//!   7. Assert refund status = succeeded
//!
//! ## Pattern
//! No Docker, no mocks, no Tilled API calls.
//! Real AR-postgres (5434) and Payments-postgres (5436).
//! Entire test is wrapped in a 60s timeout — any hang or pool starvation
//! causes the test to fail with a clear timeout message.
//!
//! ## Services required
//! - ar-postgres at localhost:5434 (AR_DATABASE_URL)
//! - payments-postgres at localhost:5436 (PAYMENTS_DATABASE_URL)
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests payments_health_canary -- --nocapture
//! ```

mod common;

use payments_rs::lifecycle::{status, transition_to_succeeded};
use serial_test::serial;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

const CANARY_TIMEOUT_SECS: u64 = 60;

// ============================================================================
// Migration helpers
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

// ============================================================================
// Seed helpers
// ============================================================================

/// Insert an AR customer and return its SERIAL id.
async fn insert_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers
             (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("canary-{}@test.local", Uuid::new_v4()))
    .bind("Payments Canary Customer")
    .fetch_one(pool)
    .await
    .expect("insert customer failed")
}

/// Insert a payment method for the customer and return its SERIAL id.
async fn insert_payment_method(pool: &PgPool, app_id: &str, customer_id: i32) -> i32 {
    let tilled_pm_id = format!("pm_canary_{}", Uuid::new_v4().simple());
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_payment_methods
             (app_id, ar_customer_id, tilled_payment_method_id, status,
              type, brand, last4, exp_month, exp_year, is_default, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 'card', 'visa', '4242', 12, 2030, TRUE, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(&tilled_pm_id)
    .fetch_one(pool)
    .await
    .expect("insert payment method failed")
}

/// Insert a payment attempt in ATTEMPTING state and return its UUID.
async fn insert_payment_attempt(pool: &PgPool, app_id: &str, invoice_ref: &str) -> Uuid {
    let payment_id = Uuid::new_v4();
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts
             (app_id, payment_id, invoice_id, attempt_no, status,
              idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, 0, 'attempting'::payment_attempt_status, $4, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_ref)
    .bind(format!("canary-idem-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert payment attempt failed")
}

/// Fetch payment attempt status as string.
async fn fetch_attempt_status(pool: &PgPool, attempt_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT status::text FROM payment_attempts WHERE id = $1")
        .bind(attempt_id)
        .fetch_one(pool)
        .await
        .expect("fetch attempt status failed")
}

/// Insert a succeeded AR charge and return its SERIAL id.
async fn insert_charge(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    tilled_charge_id: &str,
    amount_cents: i64,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_charges
             (app_id, ar_customer_id, tilled_charge_id, status, amount_cents,
              currency, charge_type, reference_id, updated_at)
         VALUES ($1, $2, $3, 'succeeded', $4, 'usd', 'one_time', $5, NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(tilled_charge_id)
    .bind(amount_cents)
    .bind(format!("ref-canary-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert charge failed")
}

/// Fetch charge status as string.
async fn fetch_charge_status(pool: &PgPool, charge_id: i32) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM ar_charges WHERE id = $1")
        .bind(charge_id)
        .fetch_one(pool)
        .await
        .expect("fetch charge status failed")
}

/// Insert a succeeded AR refund for the given charge and return its SERIAL id.
async fn insert_refund(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    charge_id: i32,
    tilled_charge_id: &str,
    amount_cents: i64,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_refunds
             (app_id, ar_customer_id, charge_id, tilled_charge_id,
              status, amount_cents, currency, reference_id, updated_at)
         VALUES ($1, $2, $3, $4, 'succeeded', $5, 'usd', $6, NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(charge_id)
    .bind(tilled_charge_id)
    .bind(amount_cents)
    .bind(format!("refund-canary-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert refund failed")
}

/// Fetch refund status as string.
async fn fetch_refund_status(pool: &PgPool, refund_id: i32) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM ar_refunds WHERE id = $1")
        .bind(refund_id)
        .fetch_one(pool)
        .await
        .expect("fetch refund status failed")
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup(ar_pool: &PgPool, payments_pool: &PgPool, app_id: &str) {
    // AR — order matters due to FK constraints
    sqlx::query("DELETE FROM ar_refunds WHERE app_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_charges WHERE app_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_payment_methods WHERE app_id = $1")
        .bind(app_id)
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
    // Payments
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id)
        .execute(payments_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(payments_pool)
        .await
        .ok();
}

// ============================================================================
// Canary test
// ============================================================================

/// Full happy-path payments canary: detects DB pool starvation or hangs within 60s.
///
/// Steps:
///   1. Create customer
///   2. Add payment method
///   3. Create payment attempt (ATTEMPTING → SUCCEEDED)
///   4. Assert attempt status = succeeded
///   5. Create AR charge (status = succeeded)
///   6. Create AR refund (status = succeeded)
///   7. Assert refund status = succeeded
#[tokio::test]
#[serial]
async fn test_payments_health_canary() {
    dotenvy::dotenv().ok();

    let result = tokio::time::timeout(Duration::from_secs(CANARY_TIMEOUT_SECS), run_canary()).await;

    match result {
        Ok(()) => println!("PASS: payments health canary completed within {}s", CANARY_TIMEOUT_SECS),
        Err(_elapsed) => panic!(
            "FAIL: payments health canary timed out after {}s — likely DB pool starvation or service hang",
            CANARY_TIMEOUT_SECS
        ),
    }
}

async fn run_canary() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;

    run_ar_migrations(&ar_pool).await;
    run_payments_migrations(&payments_pool).await;

    let app_id = common::generate_test_tenant();
    let invoice_ref = format!("inv-canary-{}", Uuid::new_v4());

    // ── Step 1: Create customer ───────────────────────────────────────────
    let customer_id = insert_customer(&ar_pool, &app_id).await;
    println!("  created customer id={}", customer_id);

    // ── Step 2: Add payment method ────────────────────────────────────────
    let pm_id = insert_payment_method(&ar_pool, &app_id, customer_id).await;
    println!("  added payment method id={}", pm_id);

    // ── Step 3: Create payment attempt (the "charge") ────────────────────
    let attempt_id = insert_payment_attempt(&payments_pool, &app_id, &invoice_ref).await;
    println!("  created payment attempt id={}", attempt_id);

    // Transition to SUCCEEDED (simulates PSP confirmation)
    transition_to_succeeded(&payments_pool, attempt_id, "canary PSP confirmed")
        .await
        .expect("transition to succeeded must work");

    // ── Step 4: Assert charge (attempt) status = succeeded ───────────────
    let attempt_status = fetch_attempt_status(&payments_pool, attempt_id).await;
    assert_eq!(
        attempt_status,
        status::SUCCEEDED,
        "payment attempt must be succeeded, got: {}",
        attempt_status
    );
    println!("  payment attempt status = {} (OK)", attempt_status);

    // ── Step 5: Create AR charge record (domain-level charge) ─────────────
    let tilled_charge_id = format!("ch_canary_{}", Uuid::new_v4().simple());
    let charge_id = insert_charge(
        &ar_pool,
        &app_id,
        customer_id,
        &tilled_charge_id,
        9900, // $99.00
    )
    .await;
    let charge_status = fetch_charge_status(&ar_pool, charge_id).await;
    assert_eq!(
        charge_status, "succeeded",
        "AR charge must be succeeded, got: {}",
        charge_status
    );
    println!("  AR charge id={} status={} (OK)", charge_id, charge_status);

    // ── Step 6: Create refund ─────────────────────────────────────────────
    let refund_id = insert_refund(
        &ar_pool,
        &app_id,
        customer_id,
        charge_id,
        &tilled_charge_id,
        9900, // full refund
    )
    .await;

    // ── Step 7: Assert refund status = succeeded ──────────────────────────
    let refund_status = fetch_refund_status(&ar_pool, refund_id).await;
    assert_eq!(
        refund_status, "succeeded",
        "refund must be succeeded, got: {}",
        refund_status
    );
    println!("  refund id={} status={} (OK)", refund_id, refund_status);

    cleanup(&ar_pool, &payments_pool, &app_id).await;

    println!("\n=== Payments health canary PASSED ===");
    println!(
        "  customer={} pm={} attempt={} charge={} refund={}",
        customer_id, pm_id, attempt_id, charge_id, refund_id
    );
}
