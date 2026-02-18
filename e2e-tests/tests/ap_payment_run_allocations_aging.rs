//! Integrated E2E: Payment run → Payments execution → allocations → bill paid + aging
//!
//! Acceptance criteria:
//! - Payment run selects correct bills deterministically
//! - Execution produces payment_id and allocations; bills become paid/partially_paid correctly
//! - Aging report reflects reduced balances immediately after payment
//! - Duplicate execution callbacks do not create duplicate allocations
//!
//! Run: ./scripts/cargo-slot.sh test -p e2e-tests -- ap_payment_run --nocapture

mod common;

use chrono::{NaiveDate, Utc};
use common::{generate_test_tenant, get_ap_pool};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use ap::domain::bills::{
    approve::approve_bill, service::create_bill, ApproveBillRequest, CreateBillLineRequest,
    CreateBillRequest,
};
use ap::domain::payment_runs::{
    builder::create_payment_run, execute::execute_payment_run, CreatePaymentRunRequest,
};
use ap::domain::reports::aging::compute_aging;

// ============================================================================
// Test helpers
// ============================================================================

async fn create_vendor(pool: &PgPool, tenant_id: &str) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
         is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(format!("E2E-Vendor-{}", &vendor_id.to_string()[..8]))
    .execute(pool)
    .await
    .expect("create vendor");
    vendor_id
}

/// Create and approve a bill so it is eligible for a payment run.
async fn create_approved_bill(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    total_minor: i64,
    due_date_str: &str,
    currency: &str,
    correlation_suffix: &str,
) -> Uuid {
    let invoice_ref = format!("INV-{}-{}", correlation_suffix, &Uuid::new_v4().to_string()[..8]);
    let bill_with_lines = create_bill(
        pool,
        tenant_id,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: invoice_ref,
            currency: currency.to_string(),
            invoice_date: Utc::now(),
            due_date: Some(
                chrono::DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", due_date_str))
                    .expect("parse due_date")
                    .with_timezone(&Utc),
            ),
            tax_minor: None,
            entered_by: "ap-clerk-e2e".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("E2E item".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: total_minor,
                gl_account_code: Some("6100".to_string()),
                po_line_id: None,
            }],
        },
        format!("corr-create-{}", correlation_suffix),
    )
    .await
    .expect("create_bill");

    let bill_id = bill_with_lines.bill.bill_id;

    approve_bill(
        pool,
        tenant_id,
        bill_id,
        &ApproveBillRequest {
            approved_by: "controller-e2e".to_string(),
            // Unmatched bills require override_reason per match policy
            override_reason: Some("e2e-unmatched-override".to_string()),
        },
        format!("corr-approve-{}", correlation_suffix),
    )
    .await
    .expect("approve_bill");

    bill_id
}

async fn cleanup(pool: &PgPool, tenant_id: &str) {
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
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Happy path: create two approved bills → build payment run → execute → both
/// bills become 'paid', allocations created, aging shows zero outstanding.
#[tokio::test]
#[serial]
async fn test_payment_run_pays_bills_and_clears_aging() {
    let pool = get_ap_pool().await;
    let tenant = generate_test_tenant();

    cleanup(&pool, &tenant).await;

    let vendor_id = create_vendor(&pool, &tenant).await;

    // Two approved USD bills due in future (current bucket)
    let bill_a = create_approved_bill(&pool, &tenant, vendor_id, 30000, "2026-03-01", "USD", "a")
        .await;
    let bill_b = create_approved_bill(&pool, &tenant, vendor_id, 20000, "2026-03-15", "USD", "b")
        .await;

    // Aging before payment: both bills in current bucket
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
    let before = compute_aging(&pool, &tenant, as_of, false)
        .await
        .expect("compute_aging before");

    assert_eq!(before.buckets_by_currency.len(), 1, "one currency bucket");
    let usd_before = &before.buckets_by_currency[0];
    assert_eq!(usd_before.currency, "USD");
    assert_eq!(
        usd_before.current_minor, 50000,
        "50000 total open before payment"
    );
    assert_eq!(usd_before.total_open_minor, 50000);

    // Create payment run
    let run_id = Uuid::new_v4();
    let run_result = create_payment_run(
        &pool,
        &tenant,
        &CreatePaymentRunRequest {
            run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "treasurer-e2e".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: Some("corr-run-happy".to_string()),
        },
    )
    .await
    .expect("create_payment_run");

    assert_eq!(run_result.run.status, "pending");
    assert_eq!(run_result.run.total_minor, 50000);
    assert_eq!(run_result.items.len(), 1, "one vendor, one item");

    let item = &run_result.items[0];
    assert_eq!(item.vendor_id, vendor_id);
    assert!(
        item.bill_ids.contains(&bill_a) && item.bill_ids.contains(&bill_b),
        "item must cover both bills"
    );

    // Execute the payment run
    let exec_result = execute_payment_run(&pool, &tenant, run_id)
        .await
        .expect("execute_payment_run");

    assert_eq!(exec_result.run.status, "completed");
    assert!(exec_result.run.executed_at.is_some());
    assert_eq!(exec_result.executions.len(), 1);
    assert_eq!(exec_result.executions[0].status, "success");
    assert_eq!(exec_result.executions[0].vendor_id, vendor_id);

    // Both bills must be 'paid'
    for bill_id in [bill_a, bill_b] {
        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
                .bind(bill_id)
                .bind(&tenant)
                .fetch_one(&pool)
                .await
                .expect("fetch bill status");
        assert_eq!(status, "paid", "bill {} must be paid after execution", bill_id);
    }

    // Allocations created: one per bill, referencing the run
    let (alloc_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ap_allocations WHERE payment_run_id = $1 AND tenant_id = $2",
    )
    .bind(run_id)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("alloc count");
    assert_eq!(alloc_count, 2, "two allocations — one per bill");

    // Payment executed event emitted
    let (ev_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.payment_executed' AND aggregate_id = $1",
    )
    .bind(run_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("event count");
    assert_eq!(ev_count, 1, "ap.payment_executed must be in outbox exactly once");

    // Aging after payment: zero outstanding (paid bills excluded)
    let after = compute_aging(&pool, &tenant, as_of, false)
        .await
        .expect("compute_aging after");
    assert!(
        after.buckets_by_currency.is_empty(),
        "aging must be empty after full payment, got {:?}",
        after.buckets_by_currency
    );

    cleanup(&pool, &tenant).await;
}

/// Idempotency: executing the same payment run twice does not create
/// duplicate allocations or change bill state incorrectly.
#[tokio::test]
#[serial]
async fn test_payment_run_execution_is_idempotent() {
    let pool = get_ap_pool().await;
    let tenant = generate_test_tenant();

    cleanup(&pool, &tenant).await;

    let vendor_id = create_vendor(&pool, &tenant).await;
    let bill_id =
        create_approved_bill(&pool, &tenant, vendor_id, 15000, "2026-03-01", "USD", "idem").await;

    let run_id = Uuid::new_v4();
    create_payment_run(
        &pool,
        &tenant,
        &CreatePaymentRunRequest {
            run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "treasurer-e2e".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: Some("corr-run-idem".to_string()),
        },
    )
    .await
    .expect("create_payment_run");

    // First execution
    let r1 = execute_payment_run(&pool, &tenant, run_id)
        .await
        .expect("first execute");
    assert_eq!(r1.run.status, "completed");

    // Second execution — idempotent, must not error
    let r2 = execute_payment_run(&pool, &tenant, run_id)
        .await
        .expect("second execute (idempotent)");
    assert_eq!(r2.run.status, "completed");
    assert_eq!(
        r1.executions[0].payment_id,
        r2.executions[0].payment_id,
        "same payment_id on retry"
    );

    // Only one allocation for the bill
    let (alloc_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("alloc count");
    assert_eq!(alloc_count, 1, "idempotent: exactly one allocation per bill");

    // Only one ap.payment_executed event
    let (ev_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.payment_executed' AND aggregate_id = $1",
    )
    .bind(run_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("event count");
    assert_eq!(ev_count, 1, "no duplicate payment_executed events");

    cleanup(&pool, &tenant).await;
}

/// Partial payment: bill with prior allocation gets remaining balance paid.
/// Aging shrinks by the allocation amount.
#[tokio::test]
#[serial]
async fn test_partial_payment_reduces_aging_balance() {
    let pool = get_ap_pool().await;
    let tenant = generate_test_tenant();

    cleanup(&pool, &tenant).await;

    let vendor_id = create_vendor(&pool, &tenant).await;

    // Create and approve a bill for 50000
    let bill_id =
        create_approved_bill(&pool, &tenant, vendor_id, 50000, "2026-03-01", "USD", "partial")
            .await;

    // Pre-allocate 20000 so open balance = 30000 and status = partially_paid
    let pre_alloc_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ap_allocations \
         (allocation_id, bill_id, tenant_id, amount_minor, currency, allocation_type, created_at) \
         VALUES ($1, $2, $3, 20000, 'USD', 'partial', NOW())",
    )
    .bind(pre_alloc_id)
    .bind(bill_id)
    .bind(&tenant)
    .execute(&pool)
    .await
    .expect("pre-allocation");

    sqlx::query(
        "UPDATE vendor_bills SET status = 'partially_paid' WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(&tenant)
    .execute(&pool)
    .await
    .expect("update to partially_paid");

    // Aging shows 30000 open
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
    let before = compute_aging(&pool, &tenant, as_of, false)
        .await
        .expect("aging before run");
    assert_eq!(before.buckets_by_currency.len(), 1);
    assert_eq!(
        before.buckets_by_currency[0].total_open_minor, 30000,
        "open balance before run must be 30000"
    );

    // Create and execute run — should pay the remaining 30000
    let run_id = Uuid::new_v4();
    create_payment_run(
        &pool,
        &tenant,
        &CreatePaymentRunRequest {
            run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "treasurer-e2e".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: Some("corr-run-partial".to_string()),
        },
    )
    .await
    .expect("create_payment_run");

    execute_payment_run(&pool, &tenant, run_id)
        .await
        .expect("execute");

    // Bill is now paid
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill_id)
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .expect("bill status");
    assert_eq!(status, "paid");

    // Execution allocation is exactly the open balance (30000)
    let (run_alloc_amount,): (i64,) = sqlx::query_as(
        "SELECT amount_minor FROM ap_allocations \
         WHERE bill_id = $1 AND payment_run_id = $2 AND tenant_id = $3",
    )
    .bind(bill_id)
    .bind(run_id)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("run allocation");
    assert_eq!(
        run_alloc_amount, 30000,
        "run must allocate only the remaining open balance"
    );

    // Aging after payment: zero outstanding
    let after = compute_aging(&pool, &tenant, as_of, false)
        .await
        .expect("aging after run");
    assert!(
        after.buckets_by_currency.is_empty(),
        "aging must be empty after full payment"
    );

    cleanup(&pool, &tenant).await;
}

/// Payment run creation with no eligible bills returns NoBillsEligible.
#[tokio::test]
#[serial]
async fn test_payment_run_no_eligible_bills_returns_error() {
    let pool = get_ap_pool().await;
    let tenant = generate_test_tenant();

    cleanup(&pool, &tenant).await;

    // No bills at all — should fail with NoBillsEligible
    let result = create_payment_run(
        &pool,
        &tenant,
        &CreatePaymentRunRequest {
            run_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "treasurer-e2e".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: None,
        },
    )
    .await;

    assert!(
        matches!(
            result,
            Err(ap::domain::payment_runs::PaymentRunError::NoBillsEligible(_, _))
        ),
        "should return NoBillsEligible, got {:?}",
        result
    );

    cleanup(&pool, &tenant).await;
}

/// Cross-tenant isolation: payment run and allocations for tenant A are
/// invisible to tenant B's aging report.
#[tokio::test]
#[serial]
async fn test_cross_tenant_isolation() {
    let pool = get_ap_pool().await;
    let tenant_a = generate_test_tenant();
    let tenant_b = generate_test_tenant();

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;

    let vendor_a = create_vendor(&pool, &tenant_a).await;
    create_approved_bill(&pool, &tenant_a, vendor_a, 25000, "2026-03-01", "USD", "isolation")
        .await;

    // Build and execute run for tenant_a
    let run_id = Uuid::new_v4();
    create_payment_run(
        &pool,
        &tenant_a,
        &CreatePaymentRunRequest {
            run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "treasurer-e2e".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: None,
        },
    )
    .await
    .expect("create run for tenant_a");

    execute_payment_run(&pool, &tenant_a, run_id)
        .await
        .expect("execute for tenant_a");

    // tenant_b must see zero aging
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
    let b_report = compute_aging(&pool, &tenant_b, as_of, false)
        .await
        .expect("aging for tenant_b");

    assert!(
        b_report.buckets_by_currency.is_empty(),
        "tenant_b must see zero aging — no cross-tenant contamination"
    );

    // tenant_a allocations must reference the run
    let (a_alloc_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ap_allocations WHERE tenant_id = $1 AND payment_run_id = $2",
    )
    .bind(&tenant_a)
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("tenant_a alloc count");
    assert_eq!(a_alloc_count, 1);

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}
