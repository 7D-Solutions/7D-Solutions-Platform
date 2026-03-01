//! E2E Test: AR Aging Projection v2 — Credit Notes + Write-offs (bd-13p, bd-22q)
//!
//! Validates that the aging projection correctly accounts for:
//!   1. Credit note reduces open balance in the correct bucket
//!   2. Write-off removes invoice from aging (balance zeroed out)
//!   3. Partial credit note reduces but does not eliminate balance
//!   4. Credit note + partial payment combined reduce balance correctly
//!   5. Refresh is idempotent with credits/write-offs applied
//!   6. Multiple credit notes on same invoice accumulate correctly
//!   7. Full credit note covers invoice, removes from aging
//!   8. Mixed scenario — multiple invoices with credits, payments, and write-offs
//!   9. **Integrated lifecycle** — usage → billing → credit note → write-off → aging → GL trial balance
//!
//! **Pattern:** No Docker, no mocks — uses live AR/GL database pools

mod common;

use anyhow::Result;
use ar_rs::aging::refresh_aging;
use ar_rs::credit_notes::{issue_credit_note, IssueCreditNoteRequest};
use ar_rs::usage_billing::{bill_usage_for_invoice, BillUsageRequest};
use ar_rs::write_offs::{write_off_invoice, WriteOffInvoiceRequest};
use chrono::{TimeZone, Utc};
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use gl_rs::consumers::gl_credit_note_consumer::{
    process_credit_note_posting, CreditNoteIssuedPayload,
};
use gl_rs::consumers::gl_writeoff_consumer::{process_writeoff_posting, InvoiceWrittenOffPayload};
use gl_rs::services::trial_balance_service::get_trial_balance;
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
    .bind(format!("aging-adj-{}@test.local", Uuid::new_v4()))
    .bind(format!("Aging Adj Test {}", tenant_id))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert an open invoice with a specific due_at
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

/// Insert a successful charge against an invoice (simulates payment)
async fn make_payment(
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

/// Issue a credit note against an invoice
async fn make_credit_note(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    invoice_id: i32,
    amount_minor: i64,
) -> Result<()> {
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
    issue_credit_note(pool, req)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

/// Write off an invoice
async fn make_write_off(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    invoice_id: i32,
    amount_minor: i64,
) -> Result<()> {
    let req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(),
        app_id: tenant_id.to_string(),
        invoice_id,
        customer_id: customer_id.to_string(),
        written_off_amount_minor: amount_minor,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: Some("test-suite".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };
    write_off_invoice(pool, req)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

/// Cleanup all tenant data (reverse FK order)
async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoice_write_offs WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_credit_notes WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_aging_buckets WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_charges WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Credit note reduces open balance in the correct aging bucket
#[tokio::test]
#[serial]
async fn test_aging_credit_note_reduces_balance() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 10000, 15 days overdue → days_1_30 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 10000, -15).await?;

    // Issue a credit note for 3000
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 3000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 10000 - 3000 = 7000
    assert_eq!(
        snapshot.days_1_30_minor, 7000,
        "Credit note should reduce 1-30 bucket: 10000 - 3000 = 7000"
    );
    assert_eq!(snapshot.total_outstanding_minor, 7000);
    assert_eq!(snapshot.invoice_count, 1);

    println!(
        "✅ Credit note reduces aging: {} minor units remaining",
        snapshot.days_1_30_minor
    );

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 2: Full write-off removes invoice from aging (balance zeroed)
#[tokio::test]
#[serial]
async fn test_aging_write_off_removes_from_aging() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 8000, 45 days overdue → days_31_60 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 8000, -45).await?;

    // A second invoice that stays open (for verification)
    make_invoice(&ar_pool, &tenant_id, customer_id, 2000, 5).await?;

    // Write off the first invoice in full
    make_write_off(&ar_pool, &tenant_id, customer_id, invoice_id, 8000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Written-off invoice should not appear in aging
    assert_eq!(
        snapshot.days_31_60_minor, 0,
        "Written-off invoice must not appear in 31-60 bucket"
    );
    assert_eq!(
        snapshot.current_minor, 2000,
        "Second invoice should still appear in current bucket"
    );
    assert_eq!(snapshot.total_outstanding_minor, 2000);
    assert_eq!(
        snapshot.invoice_count, 1,
        "Only one open invoice should remain"
    );

    println!(
        "✅ Write-off removes invoice from aging, {} minor units remaining",
        snapshot.total_outstanding_minor
    );

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 3: Partial credit note reduces but does not eliminate balance
#[tokio::test]
#[serial]
async fn test_aging_partial_credit_note() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 20000, 75 days overdue → days_61_90 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 20000, -75).await?;

    // Credit note for 5000 (partial)
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 5000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 20000 - 5000 = 15000
    assert_eq!(
        snapshot.days_61_90_minor, 15000,
        "Partial credit should leave 15000 in 61-90 bucket"
    );
    assert_eq!(snapshot.total_outstanding_minor, 15000);
    assert_eq!(snapshot.invoice_count, 1);

    println!(
        "✅ Partial credit note: {} minor units remaining in 61-90 bucket",
        snapshot.days_61_90_minor
    );

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 4: Credit note + partial payment combined reduce balance correctly
#[tokio::test]
#[serial]
async fn test_aging_credit_note_plus_payment() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 15000, 20 days overdue → days_1_30 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 15000, -20).await?;

    // Partial payment of 5000
    make_payment(&ar_pool, &tenant_id, invoice_id, customer_id, 5000).await?;

    // Credit note for 3000
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 3000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 15000 - 5000 (payment) - 3000 (credit) = 7000
    assert_eq!(
        snapshot.days_1_30_minor, 7000,
        "Payment + credit note: 15000 - 5000 - 3000 = 7000"
    );
    assert_eq!(snapshot.total_outstanding_minor, 7000);
    assert_eq!(snapshot.invoice_count, 1);

    println!(
        "✅ Payment + credit note: {} minor units remaining",
        snapshot.days_1_30_minor
    );

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 5: Refresh is idempotent with credits/write-offs applied
#[tokio::test]
#[serial]
async fn test_aging_idempotent_with_adjustments() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 12000, 50 days overdue → days_31_60 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 12000, -50).await?;

    // Credit note for 4000
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 4000).await?;

    let snapshot1 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
    let snapshot2 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Same row (upsert should have updated the existing row)
    assert_eq!(
        snapshot1.id, snapshot2.id,
        "Repeated refresh must upsert the same row"
    );
    assert_eq!(
        snapshot1.total_outstanding_minor, snapshot2.total_outstanding_minor,
        "Repeated refresh must produce same totals"
    );
    assert_eq!(
        snapshot1.days_31_60_minor, snapshot2.days_31_60_minor,
        "Repeated refresh must produce same bucket amounts"
    );
    assert_eq!(snapshot1.days_31_60_minor, 8000, "12000 - 4000 = 8000");

    println!("✅ Idempotent with adjustments: same row, same totals");

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 6: Multiple credit notes on the same invoice accumulate correctly
#[tokio::test]
#[serial]
async fn test_aging_multiple_credit_notes_accumulate() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 10000, 100 days overdue → days_over_90 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 10000, -100).await?;

    // Two credit notes: 2000 + 3000 = 5000 total credits
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 2000).await?;
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 3000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 10000 - 2000 - 3000 = 5000
    assert_eq!(
        snapshot.days_over_90_minor, 5000,
        "Two credit notes should accumulate: 10000 - 2000 - 3000 = 5000"
    );
    assert_eq!(snapshot.total_outstanding_minor, 5000);
    assert_eq!(snapshot.invoice_count, 1);

    println!(
        "✅ Multiple credit notes accumulate: {} minor units remaining",
        snapshot.days_over_90_minor
    );

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 7: Credit note that fully covers invoice removes it from aging
#[tokio::test]
#[serial]
async fn test_aging_full_credit_note_removes_from_aging() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice of 5000, 10 days overdue → days_1_30 bucket
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 5000, -10).await?;

    // A second invoice to keep aging non-empty
    make_invoice(&ar_pool, &tenant_id, customer_id, 3000, 5).await?;

    // Credit note covers full invoice amount
    make_credit_note(&ar_pool, &tenant_id, customer_id, invoice_id, 5000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    assert_eq!(
        snapshot.days_1_30_minor, 0,
        "Fully credited invoice must not appear in 1-30 bucket"
    );
    assert_eq!(
        snapshot.current_minor, 3000,
        "Second invoice in current bucket"
    );
    assert_eq!(snapshot.total_outstanding_minor, 3000);
    assert_eq!(snapshot.invoice_count, 1, "Only one open invoice counted");

    println!("✅ Full credit note removes invoice from aging");

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

/// Test 8: Mixed scenario — multiple invoices with credits, payments, and write-offs
#[tokio::test]
#[serial]
async fn test_aging_mixed_adjustments() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice A: 10000, current (due in 10 days) — credit note 2000
    let inv_a = make_invoice(&ar_pool, &tenant_id, customer_id, 10000, 10).await?;
    make_credit_note(&ar_pool, &tenant_id, customer_id, inv_a, 2000).await?;

    // Invoice B: 8000, 25 days overdue → days_1_30 — payment 3000 + credit 1000
    let inv_b = make_invoice(&ar_pool, &tenant_id, customer_id, 8000, -25).await?;
    make_payment(&ar_pool, &tenant_id, inv_b, customer_id, 3000).await?;
    make_credit_note(&ar_pool, &tenant_id, customer_id, inv_b, 1000).await?;

    // Invoice C: 6000, 50 days overdue → days_31_60 — written off in full
    let inv_c = make_invoice(&ar_pool, &tenant_id, customer_id, 6000, -50).await?;
    make_write_off(&ar_pool, &tenant_id, customer_id, inv_c, 6000).await?;

    // Invoice D: 3000, 80 days overdue → days_61_90 — no adjustments
    make_invoice(&ar_pool, &tenant_id, customer_id, 3000, -80).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Invoice A: 10000 - 2000 credit = 8000 in current
    assert_eq!(
        snapshot.current_minor, 8000,
        "Invoice A: 10000 - 2000 = 8000 current"
    );
    // Invoice B: 8000 - 3000 payment - 1000 credit = 4000 in days_1_30
    assert_eq!(
        snapshot.days_1_30_minor, 4000,
        "Invoice B: 8000 - 3000 - 1000 = 4000 days_1_30"
    );
    // Invoice C: written off, should not appear
    assert_eq!(
        snapshot.days_31_60_minor, 0,
        "Invoice C: fully written off, no balance"
    );
    // Invoice D: 3000 in days_61_90
    assert_eq!(
        snapshot.days_61_90_minor, 3000,
        "Invoice D: 3000 days_61_90"
    );
    assert_eq!(snapshot.days_over_90_minor, 0, "No invoices over 90 days");

    // Total = 8000 + 4000 + 3000 = 15000
    assert_eq!(
        snapshot.total_outstanding_minor, 15000,
        "Total: 8000 + 4000 + 3000 = 15000"
    );
    assert_eq!(snapshot.invoice_count, 3, "3 invoices with open balance");

    println!(
        "✅ Mixed adjustments: current={}, 1-30={}, 31-60={}, 61-90={}, total={}",
        snapshot.current_minor,
        snapshot.days_1_30_minor,
        snapshot.days_31_60_minor,
        snapshot.days_61_90_minor,
        snapshot.total_outstanding_minor
    );

    cleanup_tenant(&ar_pool, &tenant_id).await?;
    Ok(())
}

// ============================================================================
// Integrated Lifecycle Test (bd-22q)
// ============================================================================

/// GL account setup for the integrated test
async fn setup_gl_accounts(gl_pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
          (gen_random_uuid(), $1, 'AR',       'Accounts Receivable', 'asset',   'debit',  true),
          (gen_random_uuid(), $1, 'REV',      'Revenue',             'revenue', 'credit', true),
          (gen_random_uuid(), $1, 'BAD_DEBT', 'Bad Debt Expense',    'expense', 'debit',  true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await?;
    Ok(())
}

/// Open accounting period for Feb 2026
async fn setup_open_period(gl_pool: &sqlx::PgPool, tenant_id: &str) -> Result<uuid::Uuid> {
    let period_id = sqlx::query_scalar::<_, uuid::Uuid>(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, '2026-02-01', '2026-02-28', false)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .fetch_one(gl_pool)
    .await?;
    Ok(period_id)
}

/// Insert unbilled usage record for billing
async fn insert_usage_for_billing(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    metric_name: &str,
    quantity: f64,
    unit_price_cents: i32,
    period_start: chrono::DateTime<Utc>,
    period_end: chrono::DateTime<Utc>,
) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(&format!(
        r#"
        INSERT INTO ar_metered_usage (
            app_id, customer_id, metric_name, quantity,
            unit_price_cents, period_start, period_end, recorded_at
        )
        VALUES ($1, $2, $3, {}::NUMERIC, $4, $5, $6, NOW())
        RETURNING id
        "#,
        quantity
    ))
    .bind(tenant_id)
    .bind(customer_id)
    .bind(metric_name)
    .bind(unit_price_cents)
    .bind(period_start)
    .bind(period_end)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Test 9: **Integrated Phase 21 lifecycle** — end-to-end proof
///
/// Full chain: usage capture → bill run → verify invoice lines → issue credit note →
/// verify credit note GL posting → write off remainder → verify write-off GL posting →
/// verify aging → verify GL trial balance is balanced.
///
/// Proves: no double billing, balanced GL, accurate aging, idempotent retries.
#[tokio::test]
#[serial]
async fn test_integrated_phase21_lifecycle() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    // === SETUP ===
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    let period_id = setup_open_period(&gl_pool, &tenant_id).await?;

    let period_start = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let period_end = Utc.with_ymd_and_hms(2026, 2, 28, 23, 59, 59).unwrap();

    // Create an invoice (overdue by 10 days so it shows in 1-30 bucket)
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 0, -10).await?;

    println!("=== STEP 1: Capture usage ===");
    // Insert 2 usage records: api_calls (1000 * 10 = 10000) + storage (50 * 20 = 1000)
    insert_usage_for_billing(
        &ar_pool,
        &tenant_id,
        customer_id,
        "api_calls",
        1000.0,
        10,
        period_start,
        period_end,
    )
    .await?;
    insert_usage_for_billing(
        &ar_pool,
        &tenant_id,
        customer_id,
        "storage_gb",
        50.0,
        20,
        period_start,
        period_end,
    )
    .await?;

    let unbilled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_metered_usage WHERE app_id = $1 AND billed_at IS NULL",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(unbilled, 2, "2 unbilled usage records");
    println!("  ✅ 2 usage records captured");

    println!("=== STEP 2: Bill usage → invoice line items ===");
    let bill_result = bill_usage_for_invoice(
        &ar_pool,
        BillUsageRequest {
            app_id: tenant_id.clone(),
            invoice_id,
            customer_id,
            period_start,
            period_end,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        },
    )
    .await?;

    assert_eq!(
        bill_result.billed_count, 2,
        "Both usage records should be billed"
    );
    // api_calls: 1000 * 10 = 10000; storage: 50 * 20 = 1000; total = 11000
    assert_eq!(
        bill_result.total_amount_minor, 11000,
        "Total should be 11000 minor units"
    );

    // Verify line items created
    let line_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoice_line_items WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(line_count, 2, "2 invoice line items created");

    // Update invoice amount to reflect billed total
    sqlx::query("UPDATE ar_invoices SET amount_cents = $1 WHERE id = $2")
        .bind(11000i32)
        .bind(invoice_id)
        .execute(&ar_pool)
        .await?;

    // Verify no double billing on second call
    let second_bill = bill_usage_for_invoice(
        &ar_pool,
        BillUsageRequest {
            app_id: tenant_id.clone(),
            invoice_id,
            customer_id,
            period_start,
            period_end,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        },
    )
    .await?;
    assert_eq!(
        second_bill.billed_count, 0,
        "No double billing on second call"
    );
    println!("  ✅ 2 line items, 11000 minor, no double billing");

    println!("=== STEP 3: Issue credit note (3000 minor) ===");
    let credit_note_id = uuid::Uuid::new_v4();
    let cn_req = IssueCreditNoteRequest {
        credit_note_id,
        app_id: tenant_id.clone(),
        customer_id: customer_id.to_string(),
        invoice_id,
        amount_minor: 3000,
        currency: "usd".to_string(),
        reason: "billing_error".to_string(),
        reference_id: None,
        issued_by: Some("e2e-test".to_string()),
        correlation_id: uuid::Uuid::new_v4().to_string(),
        causation_id: None,
    };
    let cn_result = issue_credit_note(&ar_pool, cn_req)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    assert!(
        matches!(
            cn_result,
            ar_rs::credit_notes::IssueCreditNoteResult::Issued { .. }
        ),
        "Credit note should be issued"
    );
    println!("  ✅ Credit note issued: {} (3000 minor)", credit_note_id);

    println!("=== STEP 4: GL posting for credit note (DR REV 3000, CR AR 3000) ===");
    let cn_event_id = uuid::Uuid::new_v4();
    let cn_issued_at = chrono::DateTime::parse_from_rfc3339("2026-02-17T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let cn_gl_payload = CreditNoteIssuedPayload {
        credit_note_id,
        tenant_id: tenant_id.clone(),
        customer_id: "cust-e2e-integrated".to_string(),
        invoice_id: invoice_id.to_string(),
        amount_minor: 3000,
        currency: "usd".to_string(),
        reason: "billing_error".to_string(),
        issued_at: cn_issued_at,
    };

    let cn_entry_id =
        process_credit_note_posting(&gl_pool, cn_event_id, &tenant_id, "ar", &cn_gl_payload)
            .await
            .map_err(|e| anyhow::anyhow!("Credit note GL posting failed: {:?}", e))?;

    // Verify balanced: DR REV 3000, CR AR 3000
    common::assert_journal_balanced(&gl_pool, cn_entry_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Verify idempotency: same event_id rejects
    let cn_dup =
        process_credit_note_posting(&gl_pool, cn_event_id, &tenant_id, "ar", &cn_gl_payload).await;
    assert!(
        matches!(
            cn_dup,
            Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))
        ),
        "Duplicate credit note GL posting must be rejected"
    );
    println!("  ✅ GL entry {} balanced, idempotent", cn_entry_id);

    println!("=== STEP 5: Write off remaining balance (8000 minor) ===");
    let write_off_id = uuid::Uuid::new_v4();
    let wo_req = WriteOffInvoiceRequest {
        write_off_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: customer_id.to_string(),
        written_off_amount_minor: 8000,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: Some("e2e-test".to_string()),
        correlation_id: uuid::Uuid::new_v4().to_string(),
        causation_id: None,
    };
    let wo_result = write_off_invoice(&ar_pool, wo_req)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    assert!(
        matches!(
            wo_result,
            ar_rs::write_offs::WriteOffInvoiceResult::WrittenOff { .. }
        ),
        "Invoice should be written off"
    );
    println!("  ✅ Write-off issued: {} (8000 minor)", write_off_id);

    println!("=== STEP 6: GL posting for write-off (DR BAD_DEBT 8000, CR AR 8000) ===");
    let wo_event_id = uuid::Uuid::new_v4();
    let wo_gl_payload = InvoiceWrittenOffPayload {
        tenant_id: tenant_id.clone(),
        invoice_id: invoice_id.to_string(),
        customer_id: "cust-e2e-integrated".to_string(),
        written_off_amount_minor: 8000,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: Some("e2e-test".to_string()),
        written_off_at: Utc::now(),
    };

    let wo_entry_id =
        process_writeoff_posting(&gl_pool, wo_event_id, &tenant_id, "ar", &wo_gl_payload)
            .await
            .map_err(|e| anyhow::anyhow!("Write-off GL posting failed: {:?}", e))?;

    // Verify balanced: DR BAD_DEBT 8000, CR AR 8000
    common::assert_journal_balanced(&gl_pool, wo_entry_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    println!("  ✅ GL entry {} balanced", wo_entry_id);

    println!("=== STEP 7: Verify aging after credit note + write-off ===");
    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Invoice had 11000. Credit note: -3000. Write-off: -8000. Remaining: 0.
    assert_eq!(
        snapshot.total_outstanding_minor, 0,
        "After credit (3000) + write-off (8000) of 11000 invoice, outstanding should be 0"
    );
    assert_eq!(snapshot.days_1_30_minor, 0, "No balance in 1-30 bucket");
    assert_eq!(snapshot.invoice_count, 0, "No open invoices remaining");

    // Verify idempotent refresh
    let snapshot2 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
    assert_eq!(
        snapshot.total_outstanding_minor, snapshot2.total_outstanding_minor,
        "Aging refresh must be idempotent"
    );
    println!("  ✅ Aging: 0 outstanding, 0 invoices, idempotent");

    println!("=== STEP 8: Verify GL trial balance is balanced ===");
    let tb = get_trial_balance(&gl_pool, &tenant_id, period_id, "USD")
        .await
        .map_err(|e| anyhow::anyhow!("Trial balance failed: {:?}", e))?;

    assert!(tb.totals.is_balanced, "GL trial balance must be balanced");
    // Credit note: DR REV 3000, CR AR 3000
    // Write-off:   DR BAD_DEBT 8000, CR AR 8000
    // Total debits: 3000 + 8000 = 11000. Total credits: 3000 + 8000 = 11000.
    assert_eq!(
        tb.totals.total_debits, 11000,
        "Total debits should be 11000"
    );
    assert_eq!(
        tb.totals.total_credits, 11000,
        "Total credits should be 11000"
    );

    // Verify individual account balances
    let ar_row = tb.rows.iter().find(|r| r.account_code == "AR");
    let rev_row = tb.rows.iter().find(|r| r.account_code == "REV");
    let bad_debt_row = tb.rows.iter().find(|r| r.account_code == "BAD_DEBT");

    assert!(
        ar_row.is_some(),
        "AR account should appear in trial balance"
    );
    assert!(
        rev_row.is_some(),
        "REV account should appear in trial balance"
    );
    assert!(
        bad_debt_row.is_some(),
        "BAD_DEBT account should appear in trial balance"
    );

    let ar = ar_row.unwrap();
    assert_eq!(
        ar.credit_total_minor, 11000,
        "AR credits = 3000 (CN) + 8000 (WO) = 11000"
    );

    let rev = rev_row.unwrap();
    assert_eq!(
        rev.debit_total_minor, 3000,
        "REV debits = 3000 (credit note reversal)"
    );

    let bad_debt = bad_debt_row.unwrap();
    assert_eq!(
        bad_debt.debit_total_minor, 8000,
        "BAD_DEBT debits = 8000 (write-off)"
    );

    println!(
        "  ✅ Trial balance: debits={}, credits={}, balanced={}",
        tb.totals.total_debits, tb.totals.total_credits, tb.totals.is_balanced
    );

    println!("\n🎉 INTEGRATED PHASE 21 LIFECYCLE COMPLETE");
    println!("   Usage (2 records) → Bill Run (11000) → Credit Note (3000) → Write-Off (8000)");
    println!("   Aging: 0 outstanding | GL: balanced at 11000 | No double billing | Idempotent");

    // === CLEANUP ===
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

/// Test 10: Replay safety — repeated full lifecycle produces same results
///
/// Runs the full chain twice with the same tenant and verifies:
/// - No stale data leaks between runs
/// - All assertions hold on the second pass
/// - GL remains balanced after replay
#[tokio::test]
#[serial]
async fn test_integrated_lifecycle_replay_safe() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    for pass in 1..=2 {
        println!("=== REPLAY PASS {} ===", pass);

        cleanup_tenant_data(
            &ar_pool,
            &payments_pool,
            &subscriptions_pool,
            &gl_pool,
            &tenant_id,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

        let customer_id = make_customer(&ar_pool, &tenant_id).await?;
        setup_gl_accounts(&gl_pool, &tenant_id).await?;
        let period_id = setup_open_period(&gl_pool, &tenant_id).await?;

        let period_start = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
        let period_end = Utc.with_ymd_and_hms(2026, 2, 28, 23, 59, 59).unwrap();

        // Invoice overdue by 15 days
        let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 0, -15).await?;

        // Usage: 500 * 8 = 4000
        insert_usage_for_billing(
            &ar_pool,
            &tenant_id,
            customer_id,
            "compute_hours",
            500.0,
            8,
            period_start,
            period_end,
        )
        .await?;

        // Bill
        let bill_result = bill_usage_for_invoice(
            &ar_pool,
            BillUsageRequest {
                app_id: tenant_id.clone(),
                invoice_id,
                customer_id,
                period_start,
                period_end,
                correlation_id: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await?;
        assert_eq!(bill_result.billed_count, 1);
        assert_eq!(bill_result.total_amount_minor, 4000);

        sqlx::query("UPDATE ar_invoices SET amount_cents = $1 WHERE id = $2")
            .bind(4000i32)
            .bind(invoice_id)
            .execute(&ar_pool)
            .await?;

        // Credit note: 1500
        let cn_id = uuid::Uuid::new_v4();
        issue_credit_note(
            &ar_pool,
            IssueCreditNoteRequest {
                credit_note_id: cn_id,
                app_id: tenant_id.clone(),
                customer_id: customer_id.to_string(),
                invoice_id,
                amount_minor: 1500,
                currency: "usd".to_string(),
                reason: "goodwill".to_string(),
                reference_id: None,
                issued_by: Some("replay-test".to_string()),
                correlation_id: uuid::Uuid::new_v4().to_string(),
                causation_id: None,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

        // GL for credit note
        let cn_issued_at = chrono::DateTime::parse_from_rfc3339("2026-02-17T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        process_credit_note_posting(
            &gl_pool,
            uuid::Uuid::new_v4(),
            &tenant_id,
            "ar",
            &CreditNoteIssuedPayload {
                credit_note_id: cn_id,
                tenant_id: tenant_id.clone(),
                customer_id: "cust-replay".to_string(),
                invoice_id: invoice_id.to_string(),
                amount_minor: 1500,
                currency: "usd".to_string(),
                reason: "goodwill".to_string(),
                issued_at: cn_issued_at,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("CN GL posting failed pass {}: {:?}", pass, e))?;

        // Aging check: 4000 - 1500 = 2500 remaining in 1-30
        let aging = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
        assert_eq!(
            aging.days_1_30_minor, 2500,
            "Pass {}: aging should show 2500 after credit note",
            pass
        );
        assert_eq!(aging.total_outstanding_minor, 2500);

        // GL check: balanced at 1500
        let tb = get_trial_balance(&gl_pool, &tenant_id, period_id, "USD")
            .await
            .map_err(|e| anyhow::anyhow!("Trial balance failed pass {}: {:?}", pass, e))?;
        assert!(tb.totals.is_balanced, "Pass {}: GL must be balanced", pass);
        assert_eq!(tb.totals.total_debits, 1500, "Pass {}: debits = 1500", pass);

        println!(
            "  ✅ Pass {} complete: aging=2500, GL balanced at 1500",
            pass
        );
    }

    // Final cleanup
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    println!("✅ Replay safety proven: 2 passes, identical results, no stale data");
    Ok(())
}
