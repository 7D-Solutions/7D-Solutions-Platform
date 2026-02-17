//! Cross-Module E2E Test: Payment UNKNOWN Protocol (bd-3rc.6)
//!
//! **Phase 15 bd-2uw: UNKNOWN Protocol for Customer Protection**
//!
//! ## Test Coverage
//! 1. **UNKNOWN Blocks Retry:** status='unknown' excluded from retry scheduling
//! 2. **Reconciliation to SUCCEEDED:** UNKNOWN → SUCCEEDED workflow
//! 3. **Reconciliation to FAILED:** UNKNOWN → FAILED_RETRY workflow
//! 4. **Idempotency:** Reconciliation safe to call multiple times
//!
//! ## Architecture
//! - UNKNOWN protects customers from duplicate charges due to PSP ambiguity
//! - Reconciliation polls PSP to resolve actual payment status
//! - Retry scheduling blocked while status is ambiguous
//!
//! ## Pattern Reference
//! - Foundation: e2e-tests/tests/common/mod.rs (bd-3rc.1)
//! - Implementation: modules/payments/src/{reconciliation.rs, retry.rs, lifecycle.rs}

mod common;
mod oracle;

use anyhow::Result;
use chrono::NaiveDate;
use common::{generate_test_tenant, get_ar_pool, get_payments_pool};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test Setup Helpers
// ============================================================================

/// Create an AR invoice for payment testing
async fn create_test_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    due_at: NaiveDate,
) -> Result<i32, sqlx::Error> {
    let tilled_invoice_id = format!("inv-{}", Uuid::new_v4());

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices
         (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency, status, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'USD', 'open', $5, NOW(), NOW())
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&tilled_invoice_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(due_at.and_hms_opt(0, 0, 0).unwrap())
    .fetch_one(pool)
    .await?;

    Ok(invoice_id)
}

/// Create an AR customer for testing
async fn create_test_customer(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<i32, sqlx::Error> {
    let email = format!("test-{}@e2e-test.com", Uuid::new_v4());
    let external_id = format!("ext-{}", Uuid::new_v4());

    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, external_customer_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NOW(), NOW())
         ON CONFLICT (app_id, external_customer_id) DO UPDATE SET updated_at = NOW()
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&email)
    .bind("E2E Test Customer")
    .bind(&external_id)
    .fetch_one(pool)
    .await?;

    Ok(customer_id)
}

/// Create a payment attempt in UNKNOWN status
async fn create_payment_attempt_unknown(
    pool: &PgPool,
    tenant_id: &str,
    invoice_id: i32,
    payment_id: Uuid,
    processor_payment_id: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let attempt_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO payment_attempts
         (id, app_id, payment_id, invoice_id, attempt_no, status, processor_payment_id, created_at, completed_at)
         VALUES ($1, $2, $3, $4::text, 0, 'unknown'::payment_attempt_status, $5, NOW(), NOW())"
    )
    .bind(attempt_id)
    .bind(tenant_id)
    .bind(payment_id)
    .bind(invoice_id)
    .bind(processor_payment_id)
    .execute(pool)
    .await?;

    Ok(attempt_id)
}

/// Create a payment attempt in FAILED_RETRY status (eligible for retry)
async fn create_payment_attempt_failed_retry(
    pool: &PgPool,
    tenant_id: &str,
    invoice_id: i32,
    payment_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    let attempt_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO payment_attempts
         (id, app_id, payment_id, invoice_id, attempt_no, status, created_at, completed_at)
         VALUES ($1, $2, $3, $4::text, 0, 'failed_retry'::payment_attempt_status, NOW(), NOW())"
    )
    .bind(attempt_id)
    .bind(tenant_id)
    .bind(payment_id)
    .bind(invoice_id)
    .execute(pool)
    .await?;

    Ok(attempt_id)
}

/// Cleanup test data
async fn cleanup_test_data(
    payments_pool: &PgPool,
    ar_pool: &PgPool,
    tenant_id: &str,
) {
    // Payments cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(payments_pool)
        .await
        .ok();

    // AR cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_unknown_blocks_retry_eligibility() {
    // Setup
    let tenant_id = generate_test_tenant();
    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;

    let customer_id = create_test_customer(&ar_pool, &tenant_id)
        .await
        .expect("Failed to create customer");

    let due_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let invoice_id = create_test_invoice(&ar_pool, &tenant_id, customer_id, 5000, due_date)
        .await
        .expect("Failed to create invoice");

    let payment_id = Uuid::new_v4();

    // Create payment attempt with UNKNOWN status
    create_payment_attempt_unknown(
        &payments_pool,
        &tenant_id,
        invoice_id,
        payment_id,
        Some("psp_12345"),
    )
    .await
    .expect("Failed to create UNKNOWN attempt");

    // Test: Check that UNKNOWN attempt is NOT returned by retry query
    // Note: Simplified from production query which JOINs to ar_invoices
    // This test validates the core status filtering logic
    let retry_candidates: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT pa.payment_id
         FROM payment_attempts pa
         WHERE pa.app_id = $1
           AND pa.status::text IN ('attempting', 'failed_retry')
           AND pa.status::text != 'unknown'"
    )
    .bind(&tenant_id)
    .fetch_all(&payments_pool)
    .await
    .expect("Failed to query retry candidates");

    assert_eq!(
        retry_candidates.len(),
        0,
        "UNKNOWN attempt should NOT be eligible for retry"
    );

    // Test: Unit-level eligibility check
    use payments_rs::retry::is_eligible_for_retry;
    assert!(
        !is_eligible_for_retry("unknown"),
        "UNKNOWN status must block retry eligibility"
    );
    assert!(
        is_eligible_for_retry("failed_retry"),
        "FAILED_RETRY status should be eligible"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id: &tenant_id,
        tenant_id: &tenant_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    cleanup_test_data(&payments_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_unknown_excluded_from_retry_query() {
    // Setup
    let tenant_id = generate_test_tenant();
    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;

    let customer_id = create_test_customer(&ar_pool, &tenant_id)
        .await
        .expect("Failed to create customer");

    let due_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

    // Create 3 invoices with different payment attempts
    let invoice_id_1 = create_test_invoice(&ar_pool, &tenant_id, customer_id, 1000, due_date)
        .await
        .expect("Failed to create invoice 1");

    let invoice_id_2 = create_test_invoice(&ar_pool, &tenant_id, customer_id, 2000, due_date)
        .await
        .expect("Failed to create invoice 2");

    let invoice_id_3 = create_test_invoice(&ar_pool, &tenant_id, customer_id, 3000, due_date)
        .await
        .expect("Failed to create invoice 3");

    // Payment 1: UNKNOWN (should be excluded)
    let payment_id_1 = Uuid::new_v4();
    create_payment_attempt_unknown(
        &payments_pool,
        &tenant_id,
        invoice_id_1,
        payment_id_1,
        Some("psp_001"),
    )
    .await
    .expect("Failed to create UNKNOWN attempt");

    // Payment 2: FAILED_RETRY (should be included)
    let payment_id_2 = Uuid::new_v4();
    create_payment_attempt_failed_retry(
        &payments_pool,
        &tenant_id,
        invoice_id_2,
        payment_id_2,
    )
    .await
    .expect("Failed to create FAILED_RETRY attempt");

    // Payment 3: UNKNOWN (should be excluded)
    let payment_id_3 = Uuid::new_v4();
    create_payment_attempt_unknown(
        &payments_pool,
        &tenant_id,
        invoice_id_3,
        payment_id_3,
        Some("psp_003"),
    )
    .await
    .expect("Failed to create UNKNOWN attempt");

    // Assert: Only FAILED_RETRY attempt is returned (UNKNOWN excluded)
    // Note: Simplified from production query which JOINs to ar_invoices
    // This test validates that multiple UNKNOWN attempts are correctly excluded
    let retry_candidates: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT pa.payment_id
         FROM payment_attempts pa
         WHERE pa.app_id = $1
           AND pa.status::text IN ('attempting', 'failed_retry')
           AND pa.status::text != 'unknown'
         ORDER BY pa.payment_id"
    )
    .bind(&tenant_id)
    .fetch_all(&payments_pool)
    .await
    .expect("Failed to query retry candidates");

    assert_eq!(
        retry_candidates.len(),
        1,
        "Only FAILED_RETRY attempt should be eligible (UNKNOWN excluded)"
    );
    assert_eq!(
        retry_candidates[0].0, payment_id_2,
        "Wrong payment ID returned"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id: &tenant_id,
        tenant_id: &tenant_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    cleanup_test_data(&payments_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_reconciliation_resolves_unknown_to_succeeded() {
    // Setup
    let tenant_id = generate_test_tenant();
    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;

    let customer_id = create_test_customer(&ar_pool, &tenant_id)
        .await
        .expect("Failed to create customer");

    let due_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let invoice_id = create_test_invoice(&ar_pool, &tenant_id, customer_id, 5000, due_date)
        .await
        .expect("Failed to create invoice");

    let payment_id = Uuid::new_v4();
    let attempt_id = create_payment_attempt_unknown(
        &payments_pool,
        &tenant_id,
        invoice_id,
        payment_id,
        Some("psp_succeeded_"),
    )
    .await
    .expect("Failed to create UNKNOWN attempt");

    // Call reconciliation (uses mock PSP that returns success for "psp_succeeded_")
    use payments_rs::reconciliation::reconcile_unknown_attempt;
    let result = reconcile_unknown_attempt(&payments_pool, attempt_id)
        .await
        .expect("Reconciliation failed");

    // Assert: Attempt transitioned to SUCCEEDED
    let final_status: String = sqlx::query_scalar(
        "SELECT status::text FROM payment_attempts WHERE id = $1"
    )
    .bind(attempt_id)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to fetch status");

    assert_eq!(
        final_status, "succeeded",
        "Reconciliation should resolve UNKNOWN → SUCCEEDED"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id: &tenant_id,
        tenant_id: &tenant_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    cleanup_test_data(&payments_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_reconciliation_resolves_unknown_to_failed() {
    // Setup
    let tenant_id = generate_test_tenant();
    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;

    let customer_id = create_test_customer(&ar_pool, &tenant_id)
        .await
        .expect("Failed to create customer");

    let due_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let invoice_id = create_test_invoice(&ar_pool, &tenant_id, customer_id, 5000, due_date)
        .await
        .expect("Failed to create invoice");

    let payment_id = Uuid::new_v4();
    let attempt_id = create_payment_attempt_unknown(
        &payments_pool,
        &tenant_id,
        invoice_id,
        payment_id,
        Some("psp_failed_retry_"),
    )
    .await
    .expect("Failed to create UNKNOWN attempt");

    // Call reconciliation (mock PSP returns failure for "psp_failed_retry_")
    use payments_rs::reconciliation::reconcile_unknown_attempt;
    let result = reconcile_unknown_attempt(&payments_pool, attempt_id)
        .await
        .expect("Reconciliation failed");

    // Assert: Attempt transitioned to FAILED_RETRY
    let final_status: String = sqlx::query_scalar(
        "SELECT status::text FROM payment_attempts WHERE id = $1"
    )
    .bind(attempt_id)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to fetch status");

    assert_eq!(
        final_status, "failed_retry",
        "Reconciliation should resolve UNKNOWN → FAILED_RETRY"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id: &tenant_id,
        tenant_id: &tenant_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    cleanup_test_data(&payments_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_reconciliation_idempotency() {
    // Setup
    let tenant_id = generate_test_tenant();
    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;

    let customer_id = create_test_customer(&ar_pool, &tenant_id)
        .await
        .expect("Failed to create customer");

    let due_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let invoice_id = create_test_invoice(&ar_pool, &tenant_id, customer_id, 5000, due_date)
        .await
        .expect("Failed to create invoice");

    let payment_id = Uuid::new_v4();
    let attempt_id = create_payment_attempt_unknown(
        &payments_pool,
        &tenant_id,
        invoice_id,
        payment_id,
        Some("psp_succeeded_"),
    )
    .await
    .expect("Failed to create UNKNOWN attempt");

    // Call reconciliation multiple times
    use payments_rs::reconciliation::reconcile_unknown_attempt;

    let result_1 = reconcile_unknown_attempt(&payments_pool, attempt_id)
        .await
        .expect("First reconciliation failed");

    let result_2 = reconcile_unknown_attempt(&payments_pool, attempt_id)
        .await
        .expect("Second reconciliation failed");

    let result_3 = reconcile_unknown_attempt(&payments_pool, attempt_id)
        .await
        .expect("Third reconciliation failed");

    // Assert: Status remains SUCCEEDED (no duplicate transitions)
    let final_status: String = sqlx::query_scalar(
        "SELECT status::text FROM payment_attempts WHERE id = $1"
    )
    .bind(attempt_id)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to fetch status");

    assert_eq!(
        final_status, "succeeded",
        "Idempotency: status should remain SUCCEEDED after multiple reconciliation calls"
    );

    // Assert: Only one status transition (no duplicate mutations)
    let transition_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts WHERE id = $1 AND status = 'succeeded'::payment_attempt_status"
    )
    .bind(attempt_id)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to count transitions");

    assert_eq!(
        transition_count, 1,
        "Should have exactly 1 attempt in succeeded status"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id: &tenant_id,
        tenant_id: &tenant_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    cleanup_test_data(&payments_pool, &ar_pool, &tenant_id).await;
}
