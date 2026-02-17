//! E2E Test: AR Aging Correctness Under Partial Payments (bd-17w)
//!
//! Validates that the aging projection correctly incorporates payment allocation
//! rows (from ar_payment_allocations) when computing open balances:
//!
//!   1. Single invoice, partial allocation → balance reduced in correct bucket
//!   2. Multi-invoice partial payment via FIFO → oldest invoices reduced first
//!   3. Full allocation covers invoice → removed from aging (balance = 0)
//!   4. Allocation + credit note + charge combined reduce balance correctly
//!   5. Aging refresh after allocation is idempotent
//!   6. Multi-invoice across 30/60/90 buckets — verify all reconcile after partial payment
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool

mod common;

use anyhow::Result;
use ar_rs::aging::refresh_aging;
use ar_rs::payment_allocation::{allocate_payment_fifo, AllocatePaymentRequest};
use common::{cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool, get_subscriptions_pool};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Create a test customer in AR
async fn make_customer(pool: &PgPool, tenant_id: &str) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("aging-alloc-{}@test.local", Uuid::new_v4()))
    .bind(format!("Aging Alloc Test {}", tenant_id))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert an open invoice with a specific due_at offset
async fn make_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    due_offset_days: i64,
) -> Result<i32> {
    let due_at_expr = if due_offset_days >= 0 {
        format!("NOW() + INTERVAL '{} days'", due_offset_days)
    } else {
        format!("NOW() - INTERVAL '{} days'", due_offset_days.unsigned_abs())
    };

    let invoice_id: i32 = sqlx::query_scalar(&format!(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'usd', {}, NOW(), NOW())
        RETURNING id
        "#,
        due_at_expr
    ))
    .bind(tenant_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;
    Ok(invoice_id)
}

/// Insert a succeeded charge against an invoice (direct payment, not via allocation)
async fn make_charge(
    pool: &PgPool,
    tenant_id: &str,
    invoice_id: i32,
    customer_id: i32,
    amount_cents: i32,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO ar_charges (
            app_id, invoice_id, ar_customer_id, status,
            amount_cents, currency, charge_type, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'succeeded', $4, 'usd', 'one_time', NOW(), NOW())
        "#,
    )
    .bind(tenant_id)
    .bind(invoice_id)
    .bind(customer_id)
    .bind(amount_cents)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cleanup all tenant data (reverse FK order, including allocations)
async fn cleanup_tenant(
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    subscriptions_pool: &PgPool,
    gl_pool: &PgPool,
    tenant_id: &str,
) -> Result<()> {
    cleanup_tenant_data(ar_pool, payments_pool, subscriptions_pool, gl_pool, tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Issue a credit note against an invoice
async fn make_credit_note(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    invoice_id: i32,
    amount_minor: i64,
) -> Result<()> {
    use ar_rs::credit_notes::{issue_credit_note, IssueCreditNoteRequest};
    let req = IssueCreditNoteRequest {
        credit_note_id: Uuid::new_v4(),
        app_id: tenant_id.to_string(),
        customer_id: customer_id.to_string(),
        invoice_id,
        amount_minor,
        currency: "usd".to_string(),
        reason: "test_credit".to_string(),
        reference_id: None,
        issued_by: Some("test-suite".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };
    issue_credit_note(pool, req).await.map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Single invoice, partial allocation reduces balance in correct bucket
#[tokio::test]
#[serial]
async fn test_aging_partial_allocation_single_invoice() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice: 10000 cents, 15 days overdue → days_1_30 bucket
    let _inv = make_invoice(&ar_pool, &tenant_id, customer_id, 10000, -15).await?;

    // Allocate 4000 via FIFO
    let result = allocate_payment_fifo(
        &ar_pool,
        &tenant_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay_{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 4000,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    assert_eq!(result.allocated_amount_cents, 4000, "4000 should be allocated");
    assert_eq!(result.unallocated_amount_cents, 0, "No remainder");

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 10000 - 4000 allocation = 6000
    assert_eq!(snapshot.days_1_30_minor, 6000,
        "Partial allocation should reduce 1-30 bucket: 10000 - 4000 = 6000");
    assert_eq!(snapshot.total_outstanding_minor, 6000);
    assert_eq!(snapshot.invoice_count, 1);

    println!("✅ Partial allocation reduces aging: {} minor units remaining", snapshot.days_1_30_minor);

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;
    Ok(())
}

/// Test 2: Multi-invoice partial payment via FIFO → oldest invoices allocated first,
/// verify 30/60/90 buckets all reconcile
#[tokio::test]
#[serial]
async fn test_aging_multi_invoice_partial_payment_fifo() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Create 4 invoices in different aging buckets:
    // Invoice A: 5000, 100 days overdue → days_over_90 (oldest due, allocated first by FIFO)
    let _inv_a = make_invoice(&ar_pool, &tenant_id, customer_id, 5000, -100).await?;
    // Invoice B: 8000, 75 days overdue → days_61_90
    let _inv_b = make_invoice(&ar_pool, &tenant_id, customer_id, 8000, -75).await?;
    // Invoice C: 6000, 45 days overdue → days_31_60
    let _inv_c = make_invoice(&ar_pool, &tenant_id, customer_id, 6000, -45).await?;
    // Invoice D: 3000, 10 days overdue → days_1_30
    let _inv_d = make_invoice(&ar_pool, &tenant_id, customer_id, 3000, -10).await?;

    // Before allocation: verify baseline aging
    let baseline = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
    assert_eq!(baseline.days_over_90_minor, 5000, "Baseline: 5000 in 90+");
    assert_eq!(baseline.days_61_90_minor, 8000, "Baseline: 8000 in 61-90");
    assert_eq!(baseline.days_31_60_minor, 6000, "Baseline: 6000 in 31-60");
    assert_eq!(baseline.days_1_30_minor, 3000, "Baseline: 3000 in 1-30");
    assert_eq!(baseline.total_outstanding_minor, 22000, "Baseline total: 22000");

    println!("  Baseline: 90+={}, 61-90={}, 31-60={}, 1-30={}, total={}",
        baseline.days_over_90_minor, baseline.days_61_90_minor,
        baseline.days_31_60_minor, baseline.days_1_30_minor,
        baseline.total_outstanding_minor);

    // Allocate 9000 via FIFO: should cover Invoice A (5000) fully + Invoice B (4000 partial)
    let result = allocate_payment_fifo(
        &ar_pool,
        &tenant_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay_{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 9000,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    assert_eq!(result.allocated_amount_cents, 9000, "Full 9000 should be allocated");
    assert_eq!(result.unallocated_amount_cents, 0);
    assert_eq!(result.allocations.len(), 2, "Should allocate to 2 invoices (A fully, B partially)");

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Invoice A: 5000 - 5000 = 0 (fully allocated, removed from aging)
    assert_eq!(snapshot.days_over_90_minor, 0,
        "Invoice A fully allocated: 5000 - 5000 = 0 in 90+ bucket");
    // Invoice B: 8000 - 4000 = 4000 remaining
    assert_eq!(snapshot.days_61_90_minor, 4000,
        "Invoice B partially allocated: 8000 - 4000 = 4000 in 61-90 bucket");
    // Invoice C: unchanged at 6000
    assert_eq!(snapshot.days_31_60_minor, 6000,
        "Invoice C unchanged: 6000 in 31-60 bucket");
    // Invoice D: unchanged at 3000
    assert_eq!(snapshot.days_1_30_minor, 3000,
        "Invoice D unchanged: 3000 in 1-30 bucket");
    // Total: 0 + 4000 + 6000 + 3000 = 13000
    assert_eq!(snapshot.total_outstanding_minor, 13000,
        "Total after allocation: 22000 - 9000 = 13000");
    assert_eq!(snapshot.invoice_count, 3,
        "3 invoices with open balance (Invoice A fully covered)");

    println!("✅ Multi-invoice FIFO allocation: 90+={}, 61-90={}, 31-60={}, 1-30={}, total={}",
        snapshot.days_over_90_minor, snapshot.days_61_90_minor,
        snapshot.days_31_60_minor, snapshot.days_1_30_minor,
        snapshot.total_outstanding_minor);

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;
    Ok(())
}

/// Test 3: Full allocation covers invoice → removed from aging
#[tokio::test]
#[serial]
async fn test_aging_full_allocation_removes_from_aging() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice: 7000, 50 days overdue → days_31_60
    let _inv = make_invoice(&ar_pool, &tenant_id, customer_id, 7000, -50).await?;
    // Second invoice to keep aging non-empty
    let _inv2 = make_invoice(&ar_pool, &tenant_id, customer_id, 2000, 5).await?;

    // Allocate full amount
    allocate_payment_fifo(
        &ar_pool,
        &tenant_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay_{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 7000,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    assert_eq!(snapshot.days_31_60_minor, 0,
        "Fully allocated invoice must not appear in 31-60 bucket");
    assert_eq!(snapshot.current_minor, 2000,
        "Second invoice should still appear in current bucket");
    assert_eq!(snapshot.total_outstanding_minor, 2000);
    assert_eq!(snapshot.invoice_count, 1, "Only 1 open invoice remaining");

    println!("✅ Full allocation removes invoice from aging, {} outstanding", snapshot.total_outstanding_minor);

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;
    Ok(())
}

/// Test 4: Allocation + credit note + charge combined reduce balance correctly
#[tokio::test]
#[serial]
async fn test_aging_allocation_plus_credit_note_plus_charge() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice: 20000, 20 days overdue → days_1_30
    let inv = make_invoice(&ar_pool, &tenant_id, customer_id, 20000, -20).await?;

    // Direct charge: 3000
    make_charge(&ar_pool, &tenant_id, inv, customer_id, 3000).await?;

    // Credit note: 2000
    make_credit_note(&ar_pool, &tenant_id, customer_id, inv, 2000).await?;

    // Allocation: 5000
    allocate_payment_fifo(
        &ar_pool,
        &tenant_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay_{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 5000,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 20000 - 3000 (charge) - 5000 (allocation) - 2000 (credit) = 10000
    assert_eq!(snapshot.days_1_30_minor, 10000,
        "Combined: 20000 - 3000 charge - 5000 alloc - 2000 credit = 10000");
    assert_eq!(snapshot.total_outstanding_minor, 10000);
    assert_eq!(snapshot.invoice_count, 1);

    println!("✅ Charge + allocation + credit note combined: {} remaining", snapshot.days_1_30_minor);

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;
    Ok(())
}

/// Test 5: Aging refresh after allocation is idempotent
#[tokio::test]
#[serial]
async fn test_aging_allocation_idempotent_refresh() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice: 15000, 65 days overdue → days_61_90
    let _inv = make_invoice(&ar_pool, &tenant_id, customer_id, 15000, -65).await?;

    // Allocate 6000
    allocate_payment_fifo(
        &ar_pool,
        &tenant_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay_{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 6000,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    let snap1 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
    let snap2 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    assert_eq!(snap1.id, snap2.id, "Repeated refresh must upsert same row");
    assert_eq!(snap1.days_61_90_minor, 9000, "15000 - 6000 = 9000");
    assert_eq!(snap1.total_outstanding_minor, snap2.total_outstanding_minor,
        "Totals must match across refreshes");
    assert_eq!(snap1.days_61_90_minor, snap2.days_61_90_minor,
        "Bucket amounts must match across refreshes");

    println!("✅ Idempotent: same row, same totals after repeated refresh");

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;
    Ok(())
}

/// Test 6: Multi-invoice across all aging buckets — verify reconciliation after
/// a partial payment that spans multiple invoices via FIFO allocation
///
/// This is the primary acceptance test: creates invoices in current, 1-30, 31-60,
/// 61-90, and 90+ buckets, applies a partial FIFO payment, then verifies every
/// bucket reconciles to (original balance − allocated amount).
#[tokio::test]
#[serial]
async fn test_aging_multi_bucket_reconciliation() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // 5 invoices, one per bucket (oldest due_at first for FIFO ordering):
    // E: 2000,  95 days overdue → over_90  (FIFO order 1)
    let _inv_e = make_invoice(&ar_pool, &tenant_id, customer_id, 2000, -95).await?;
    // D: 4000,  70 days overdue → 61-90    (FIFO order 2)
    let _inv_d = make_invoice(&ar_pool, &tenant_id, customer_id, 4000, -70).await?;
    // C: 3000,  40 days overdue → 31-60    (FIFO order 3)
    let _inv_c = make_invoice(&ar_pool, &tenant_id, customer_id, 3000, -40).await?;
    // B: 5000,  15 days overdue → 1-30     (FIFO order 4)
    let _inv_b = make_invoice(&ar_pool, &tenant_id, customer_id, 5000, -15).await?;
    // A: 6000,  due in 10 days → current   (FIFO order 5)
    let _inv_a = make_invoice(&ar_pool, &tenant_id, customer_id, 6000, 10).await?;

    // Total: 2000 + 4000 + 3000 + 5000 + 6000 = 20000
    let baseline = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
    assert_eq!(baseline.total_outstanding_minor, 20000, "Baseline total: 20000");
    println!("  Baseline: over_90={}, 61-90={}, 31-60={}, 1-30={}, current={}, total={}",
        baseline.days_over_90_minor, baseline.days_61_90_minor,
        baseline.days_31_60_minor, baseline.days_1_30_minor,
        baseline.current_minor, baseline.total_outstanding_minor);

    // Allocate 7500 via FIFO:
    // E: 2000 fully allocated (0 remaining)   → over_90 = 0
    // D: 4000 fully allocated (0 remaining)   → 61-90 = 0
    // C: 1500 partial (3000 - 1500 = 1500)    → 31-60 = 1500
    // B: untouched                             → 1-30 = 5000
    // A: untouched                             → current = 6000
    let result = allocate_payment_fifo(
        &ar_pool,
        &tenant_id,
        &AllocatePaymentRequest {
            payment_id: format!("pay_{}", Uuid::new_v4()),
            customer_id,
            amount_cents: 7500,
            currency: "usd".to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    assert_eq!(result.allocated_amount_cents, 7500, "Full 7500 allocated");
    assert_eq!(result.unallocated_amount_cents, 0);

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Verify each bucket reconciles
    assert_eq!(snapshot.days_over_90_minor, 0,
        "Invoice E fully allocated: 2000 - 2000 = 0");
    assert_eq!(snapshot.days_61_90_minor, 0,
        "Invoice D fully allocated: 4000 - 4000 = 0");
    assert_eq!(snapshot.days_31_60_minor, 1500,
        "Invoice C partially allocated: 3000 - 1500 = 1500");
    assert_eq!(snapshot.days_1_30_minor, 5000,
        "Invoice B untouched: 5000");
    assert_eq!(snapshot.current_minor, 6000,
        "Invoice A untouched: 6000");

    // Total: 0 + 0 + 1500 + 5000 + 6000 = 12500
    assert_eq!(snapshot.total_outstanding_minor, 12500,
        "Total after allocation: 20000 - 7500 = 12500");
    assert_eq!(snapshot.invoice_count, 3,
        "3 invoices with open balance (E and D fully covered)");

    // Verify reconciliation: total = sum of all buckets
    let bucket_sum = snapshot.current_minor
        + snapshot.days_1_30_minor
        + snapshot.days_31_60_minor
        + snapshot.days_61_90_minor
        + snapshot.days_over_90_minor;
    assert_eq!(bucket_sum, snapshot.total_outstanding_minor,
        "Bucket sum must equal total outstanding (reconciliation check)");

    println!("✅ Multi-bucket reconciliation after FIFO allocation:");
    println!("   over_90={}, 61-90={}, 31-60={}, 1-30={}, current={}, total={}",
        snapshot.days_over_90_minor, snapshot.days_61_90_minor,
        snapshot.days_31_60_minor, snapshot.days_1_30_minor,
        snapshot.current_minor, snapshot.total_outstanding_minor);
    println!("   Bucket sum ({}) = total ({}): ✓", bucket_sum, snapshot.total_outstanding_minor);

    cleanup_tenant(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id).await?;
    Ok(())
}
