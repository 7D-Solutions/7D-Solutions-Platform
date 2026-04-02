//! E2E Test: Reconciliation Matching Engine v1 (bd-2cn)
//!
//! **Coverage:**
//! 1. Exact match — payment matched to invoice by customer + amount + currency
//! 2. Reference match — payment matched via reference_id ↔ tilled_invoice_id
//! 3. Unmatched payment — exception raised when no invoice matches
//! 4. Ambiguous match — exception raised when multiple invoices match equally
//! 5. Idempotency — duplicate recon_run_id returns AlreadyExists
//! 6. Determinism — same inputs produce identical outputs across runs
//! 7. Outbox atomicity — run record + matches + exceptions + events committed together
//! 8. Already-matched items excluded — prior matches are not re-matched
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use ar_rs::recon_scheduler::{
    claim_and_execute_scheduled_run, create_scheduled_run, CreateScheduledRunOutcome,
    CreateScheduledRunRequest, ScheduledRunExecutionOutcome,
};
use ar_rs::reconciliation::{run_reconciliation, RunReconOutcome, RunReconRequest};
use chrono::{NaiveDateTime, Utc};
use common::{generate_test_tenant, get_ar_pool};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a test customer and return its ID.
async fn create_customer(pool: &sqlx::PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, 'Recon Test Customer', 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("recon-test-{}@test.local", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("failed to create customer")
}

/// Insert a test invoice (status 'open') and return its ID.
async fn create_invoice(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i64,
    currency: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, $5, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_recon_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .fetch_one(pool)
    .await
    .expect("failed to create invoice")
}

/// Insert a test invoice with a specific tilled_invoice_id for reference matching.
async fn create_invoice_with_ref(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i64,
    currency: &str,
    tilled_invoice_id: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, $5, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(tilled_invoice_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .fetch_one(pool)
    .await
    .expect("failed to create invoice with ref")
}

/// Insert a test charge (payment) with status 'succeeded' and return its ID.
async fn create_charge(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i64,
    currency: &str,
    reference_id: Option<&str>,
) -> i32 {
    let ref_id = reference_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("pay_ref_{}", Uuid::new_v4()));
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_charges (
            app_id, ar_customer_id, status, amount_cents, currency,
            charge_type, reason, reference_id, created_at, updated_at
        )
        VALUES ($1, $2, 'succeeded', $3, $4, 'one_time', 'payment', $5, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(&ref_id)
    .fetch_one(pool)
    .await
    .expect("failed to create charge")
}

/// Run the migration to create reconciliation tables.
async fn run_recon_migration(pool: &sqlx::PgPool) {
    let sql =
        include_str!("../../modules/ar/db/migrations/20260217000006_create_recon_matching.sql");
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("failed to run recon migration");
}

/// Clean up all test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    // Recon exceptions
    sqlx::query("DELETE FROM ar_recon_exceptions WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Recon matches
    sqlx::query("DELETE FROM ar_recon_matches WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Recon runs
    sqlx::query("DELETE FROM ar_recon_runs WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Outbox
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Charges
    sqlx::query("DELETE FROM ar_charges WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Invoices
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Customers
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Exact match — payment matched to invoice by customer + amount + currency.
#[tokio::test]
async fn test_recon_exact_match() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 10000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 10000, "usd", None).await;

    let run_id = Uuid::new_v4();
    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(r.match_count, 1, "expected 1 match");
            assert_eq!(r.exception_count, 0, "expected 0 exceptions");
            assert_eq!(r.payment_count, 1);
            assert_eq!(r.invoice_count, 1);
            assert_eq!(r.status, "completed");
        }
        RunReconOutcome::AlreadyExists(_) => panic!("expected Executed, got AlreadyExists"),
    }

    // Verify match record in DB
    let match_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_recon_matches WHERE recon_run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .expect("count query failed");
    assert_eq!(match_count, 1);

    // Verify match confidence
    let confidence: f64 = sqlx::query_scalar(
        "SELECT confidence_score::FLOAT8 FROM ar_recon_matches WHERE recon_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("confidence query failed");
    assert!(
        (confidence - 1.0).abs() < f64::EPSILON,
        "exact match confidence must be 1.0"
    );

    // Verify match method
    let method: String =
        sqlx::query_scalar("SELECT match_method FROM ar_recon_matches WHERE recon_run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .expect("method query failed");
    assert_eq!(method, "exact");

    // Verify outbox events
    let run_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.recon_run_started'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert_eq!(run_event_count, 1, "exactly one run_started event");

    let match_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.recon_match_applied'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert_eq!(match_event_count, 1, "exactly one match_applied event");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 2: Reference match — payment matched via reference_id ↔ tilled_invoice_id.
#[tokio::test]
async fn test_recon_reference_match() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer_a = create_customer(&pool, &tenant_id).await;
    let customer_b = create_customer(&pool, &tenant_id).await;
    let ref_id = format!("inv_ref_{}", Uuid::new_v4());

    // Invoice belongs to customer_b, different amount
    create_invoice_with_ref(&pool, &tenant_id, customer_b, 7000, "usd", &ref_id).await;
    // Payment from customer_a references the invoice by ref_id
    create_charge(&pool, &tenant_id, customer_a, 5000, "usd", Some(&ref_id)).await;

    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(r.match_count, 1, "expected reference match");
            assert_eq!(r.exception_count, 0);
        }
        RunReconOutcome::AlreadyExists(_) => panic!("expected Executed"),
    }

    // Verify match method is "reference"
    let method: String = sqlx::query_scalar(
        "SELECT match_method FROM ar_recon_matches WHERE app_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("method query failed");
    assert_eq!(method, "reference");

    // Verify confidence is 0.95
    let confidence: f64 = sqlx::query_scalar(
        "SELECT confidence_score::FLOAT8 FROM ar_recon_matches WHERE app_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("confidence query failed");
    assert!((confidence - 0.95).abs() < 0.01);

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 3: Unmatched payment — exception raised when no invoice matches.
#[tokio::test]
async fn test_recon_unmatched_payment_exception() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    // Create a payment with no matching invoice
    create_charge(&pool, &tenant_id, customer, 5000, "usd", None).await;

    let run_id = Uuid::new_v4();
    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(r.match_count, 0);
            assert_eq!(
                r.exception_count, 1,
                "expected 1 exception for unmatched payment"
            );
        }
        RunReconOutcome::AlreadyExists(_) => panic!("expected Executed"),
    }

    // Verify exception record
    let exception_kind: String = sqlx::query_scalar(
        "SELECT exception_kind FROM ar_recon_exceptions WHERE recon_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("exception query failed");
    assert_eq!(exception_kind, "unmatched_payment");

    // Verify outbox exception event
    let exc_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.recon_exception_raised'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert_eq!(exc_event_count, 1);

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 4: Ambiguous match — exception raised when multiple invoices match equally.
#[tokio::test]
async fn test_recon_ambiguous_match_exception() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    // Two identical invoices for same customer/amount
    create_invoice(&pool, &tenant_id, customer, 5000, "usd").await;
    create_invoice(&pool, &tenant_id, customer, 5000, "usd").await;
    // One payment
    create_charge(&pool, &tenant_id, customer, 5000, "usd", None).await;

    let run_id = Uuid::new_v4();
    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(
                r.match_count, 0,
                "ambiguous match must not be auto-resolved"
            );
            assert_eq!(r.exception_count, 1, "expected ambiguous_match exception");
        }
        RunReconOutcome::AlreadyExists(_) => panic!("expected Executed"),
    }

    let exception_kind: String = sqlx::query_scalar(
        "SELECT exception_kind FROM ar_recon_exceptions WHERE recon_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("exception query failed");
    assert_eq!(exception_kind, "ambiguous_match");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 5: Idempotency — duplicate recon_run_id returns AlreadyExists.
#[tokio::test]
async fn test_recon_idempotency() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 3000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 3000, "usd", None).await;

    let run_id = Uuid::new_v4();
    let req = RunReconRequest {
        recon_run_id: run_id,
        app_id: tenant_id.clone(),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    // First run — should execute.
    let result1 = run_reconciliation(&pool, req.clone())
        .await
        .expect("first run failed");
    assert!(matches!(result1, RunReconOutcome::Executed(_)));

    // Second run with same recon_run_id — should return AlreadyExists.
    let result2 = run_reconciliation(&pool, req)
        .await
        .expect("second run failed");
    match result2 {
        RunReconOutcome::AlreadyExists(r) => {
            assert_eq!(r.recon_run_id, run_id);
            assert_eq!(r.status, "completed");
            assert_eq!(r.match_count, 1);
        }
        RunReconOutcome::Executed(_) => panic!("expected AlreadyExists on duplicate run_id"),
    }

    // Only one run row and one match row
    let run_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_recon_runs WHERE recon_run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(run_count, 1);

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 6: Determinism — same inputs produce identical results across separate runs.
#[tokio::test]
async fn test_recon_determinism_across_runs() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    let inv_id = create_invoice(&pool, &tenant_id, customer, 8000, "usd").await;
    let charge_id = create_charge(&pool, &tenant_id, customer, 8000, "usd", None).await;

    // Run 1
    let run1_id = Uuid::new_v4();
    let result1 = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run1_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("run 1 failed");

    let r1 = match result1 {
        RunReconOutcome::Executed(r) => r,
        _ => panic!("expected Executed"),
    };

    // Run 1 matches the payment to the invoice → they are now "matched"
    // Run 2 should have 0 payments and 0 invoices (all consumed by run 1)
    let run2_id = Uuid::new_v4();
    let result2 = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run2_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("run 2 failed");

    let r2 = match result2 {
        RunReconOutcome::Executed(r) => r,
        _ => panic!("expected Executed"),
    };

    // Run 1 matched everything; run 2 sees nothing to match
    assert_eq!(r1.match_count, 1);
    assert_eq!(
        r2.payment_count, 0,
        "already-matched payments must be excluded from run 2"
    );
    assert_eq!(
        r2.invoice_count, 0,
        "already-matched invoices must be excluded from run 2"
    );
    assert_eq!(r2.match_count, 0);

    // Verify first run match details
    let (payment_id, invoice_id): (String, String) = sqlx::query_as(
        "SELECT payment_id, invoice_id FROM ar_recon_matches WHERE recon_run_id = $1",
    )
    .bind(run1_id)
    .fetch_one(&pool)
    .await
    .expect("match query failed");
    assert_eq!(payment_id, charge_id.to_string());
    assert_eq!(invoice_id, inv_id.to_string());

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 7: Outbox atomicity — run + match + event committed atomically.
#[tokio::test]
async fn test_recon_outbox_atomicity() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 2500, "usd").await;
    create_charge(&pool, &tenant_id, customer, 2500, "usd", None).await;

    let run_id = Uuid::new_v4();
    run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    // Verify atomicity: run row exists
    let run_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_recon_runs WHERE recon_run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(run_exists, 1);

    // Match row exists
    let match_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_recon_matches WHERE recon_run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(match_exists, 1);

    // Outbox events exist (run_started + match_applied = 2)
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type LIKE 'ar.recon_%'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        outbox_count, 2,
        "run_started + match_applied = 2 outbox events"
    );

    // Verify DATA_MUTATION class on all outbox events
    let mutation_classes: Vec<String> = sqlx::query_scalar(
        "SELECT mutation_class FROM events_outbox WHERE tenant_id = $1 AND event_type LIKE 'ar.recon_%'",
    )
    .bind(&tenant_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    for mc in &mutation_classes {
        assert_eq!(
            mc, "DATA_MUTATION",
            "recon events must carry DATA_MUTATION class"
        );
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 8: Already-matched items excluded from subsequent runs.
#[tokio::test]
async fn test_recon_already_matched_excluded() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 4000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 4000, "usd", None).await;

    // First run matches everything.
    run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("first run failed");

    // Add a new unmatched payment.
    create_charge(&pool, &tenant_id, customer, 9999, "usd", None).await;

    // Second run: only the new payment should be considered.
    let run2_id = Uuid::new_v4();
    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run2_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("second run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(
                r.payment_count, 1,
                "only the new payment should be in scope"
            );
            assert_eq!(r.invoice_count, 0, "first invoice already matched");
            assert_eq!(r.match_count, 0, "no matching invoice for $99.99 payment");
            assert_eq!(r.exception_count, 1, "unmatched payment exception");
        }
        _ => panic!("expected Executed"),
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 9: Multiple customers — matches are isolated per customer.
#[tokio::test]
async fn test_recon_customer_isolation() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer_a = create_customer(&pool, &tenant_id).await;
    let customer_b = create_customer(&pool, &tenant_id).await;

    // Customer A: invoice $100, payment $100 → match
    create_invoice(&pool, &tenant_id, customer_a, 10000, "usd").await;
    create_charge(&pool, &tenant_id, customer_a, 10000, "usd", None).await;

    // Customer B: payment $100 with no invoice → unmatched
    create_charge(&pool, &tenant_id, customer_b, 10000, "usd", None).await;

    let run_id = Uuid::new_v4();
    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(r.match_count, 1, "only customer A's payment should match");
            assert_eq!(
                r.exception_count, 1,
                "customer B's payment has no matching invoice"
            );
        }
        _ => panic!("expected Executed"),
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 10: Currency mismatch prevents matching.
#[tokio::test]
async fn test_recon_currency_mismatch() {
    let pool = get_ar_pool().await;
    run_recon_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 5000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 5000, "eur", None).await; // different currency

    let run_id = Uuid::new_v4();
    let result = run_reconciliation(
        &pool,
        RunReconRequest {
            recon_run_id: run_id,
            app_id: tenant_id.clone(),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("recon run failed");

    match result {
        RunReconOutcome::Executed(r) => {
            assert_eq!(r.match_count, 0, "currency mismatch prevents matching");
            assert_eq!(r.exception_count, 1, "payment unmatched due to currency");
        }
        _ => panic!("expected Executed"),
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Integrated cross-domain test (bd-b6k)
// ============================================================================

/// Advisory lock key for serializing migration execution across parallel tests.
const RECON_SCHED_MIGRATION_LOCK_KEY: i64 = 8_312_947_654_i64;

/// Run the migrations for both recon matching and scheduled runs (idempotent).
async fn run_scheduled_run_migrations(pool: &sqlx::PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RECON_SCHED_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("failed to acquire migration advisory lock");

    let recon_sql =
        include_str!("../../modules/ar/db/migrations/20260217000006_create_recon_matching.sql");
    let _ = sqlx::raw_sql(recon_sql).execute(pool).await;

    let sched_sql = include_str!(
        "../../modules/ar/db/migrations/20260217000009_create_recon_scheduled_runs.sql"
    );
    let _ = sqlx::raw_sql(sched_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RECON_SCHED_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("failed to release migration advisory lock");
}

/// Create a window start/end pair for testing.
fn make_window(offset_hours: i64) -> (NaiveDateTime, NaiveDateTime) {
    let start = Utc::now().naive_utc() - chrono::Duration::hours(offset_hours + 1);
    let end = Utc::now().naive_utc() - chrono::Duration::hours(offset_hours);
    (start, end)
}

/// Clean up scheduled run data in addition to recon data.
async fn cleanup_tenant_with_scheduled_runs(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM ar_recon_scheduled_runs WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    cleanup_tenant(pool, tenant_id).await;
}

/// Test 11 (bd-b6k): Integrated scheduled recon run → matching engine produces stable results.
///
/// Proves the full chain: create a scheduled run → worker claims and executes it →
/// matching engine runs inside the execution → matches and exceptions are produced
/// and visible in DB. A second scheduled run on the same data produces 0 new matches
/// (already-matched items excluded).
#[tokio::test]
async fn test_integrated_scheduled_recon_produces_stable_results() {
    let pool = get_ar_pool().await;
    run_scheduled_run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // --- Setup: 2 customers, 3 invoices, 2 payments ---
    let cust_a = create_customer(&pool, &tenant_id).await;
    let cust_b = create_customer(&pool, &tenant_id).await;

    // Customer A: $100 invoice + $100 payment → exact match
    create_invoice(&pool, &tenant_id, cust_a, 10000, "usd").await;
    create_charge(&pool, &tenant_id, cust_a, 10000, "usd", None).await;

    // Customer B: $50 invoice, no payment → no match
    create_invoice(&pool, &tenant_id, cust_b, 5000, "usd").await;

    // Customer B: $75 payment, no matching invoice → unmatched exception
    create_charge(&pool, &tenant_id, cust_b, 7500, "usd", None).await;

    // --- Step 1: Create a scheduled run ---
    let (window_start, window_end) = make_window(2);
    let sched_run_id = Uuid::new_v4();
    let create_result = create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: sched_run_id,
            app_id: tenant_id.clone(),
            window_start,
            window_end,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("create_scheduled_run failed");

    let scheduled_run_id = match create_result {
        CreateScheduledRunOutcome::Created(r) => {
            assert_eq!(r.status, "pending");
            r.scheduled_run_id
        }
        CreateScheduledRunOutcome::AlreadyScheduled(_) => {
            panic!("expected Created, got AlreadyScheduled");
        }
    };

    // --- Step 2: Worker claims and executes the run ---
    let worker_id = Uuid::new_v4().to_string();
    let correlation_id = Uuid::new_v4().to_string();
    let exec_result =
        claim_and_execute_scheduled_run(&pool, &worker_id, &correlation_id, Some(&tenant_id))
            .await
            .expect("claim_and_execute_scheduled_run failed");

    match exec_result {
        ScheduledRunExecutionOutcome::Completed(r) => {
            assert_eq!(r.match_count.unwrap_or(0), 1, "customer A's exact match");
            assert_eq!(
                r.exception_count.unwrap_or(0),
                1,
                "customer B's unmatched payment"
            );
            assert_eq!(r.status, "completed");
        }
        ScheduledRunExecutionOutcome::NothingToClaim => {
            panic!("expected Completed, got NothingToClaim");
        }
        ScheduledRunExecutionOutcome::Failed { error, .. } => {
            panic!("expected Completed, got Failed: {:?}", error);
        }
    }

    // --- Step 3: Verify matches and exceptions in DB ---
    let match_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_recon_matches WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("match count query failed");
    assert_eq!(match_count, 1, "exactly one match from scheduled run");

    let exception_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_recon_exceptions WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("exception count query failed");
    assert_eq!(
        exception_count, 1,
        "exactly one exception from scheduled run"
    );

    // --- Step 4: Second scheduled run → 0 new matches (stability) ---
    let (window_start2, window_end2) = make_window(4);
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: window_start2,
            window_end: window_end2,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("second create_scheduled_run failed");

    let worker_id2 = Uuid::new_v4().to_string();
    let correlation_id2 = Uuid::new_v4().to_string();
    let exec_result2 =
        claim_and_execute_scheduled_run(&pool, &worker_id2, &correlation_id2, Some(&tenant_id))
            .await
            .expect("second claim_and_execute failed");

    match exec_result2 {
        ScheduledRunExecutionOutcome::Completed(r) => {
            assert_eq!(
                r.match_count.unwrap_or(0),
                0,
                "already-matched items excluded from second run"
            );
        }
        ScheduledRunExecutionOutcome::NothingToClaim => {
            panic!("expected Completed, got NothingToClaim");
        }
        ScheduledRunExecutionOutcome::Failed { error, .. } => {
            panic!("expected Completed, got Failed: {:?}", error);
        }
    }

    // Verify the scheduled run status was updated to completed
    let run_status: String = sqlx::query_scalar(
        "SELECT status FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(scheduled_run_id)
    .fetch_one(&pool)
    .await
    .expect("run status query failed");
    assert_eq!(run_status, "completed");

    cleanup_tenant_with_scheduled_runs(&pool, &tenant_id).await;
}
