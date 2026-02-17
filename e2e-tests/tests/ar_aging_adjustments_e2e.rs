//! E2E Test: AR Aging Projection v2 — Credit Notes + Write-offs (bd-13p)
//!
//! Validates that the aging projection correctly accounts for:
//!   1. Credit note reduces open balance in the correct bucket
//!   2. Write-off removes invoice from aging (balance zeroed out)
//!   3. Partial credit note reduces but does not eliminate balance
//!   4. Credit note + partial payment combined reduce balance correctly
//!   5. Refresh is idempotent with credits/write-offs applied
//!   6. Multiple credit notes on same invoice accumulate correctly
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use anyhow::Result;
use ar_rs::aging::refresh_aging;
use ar_rs::credit_notes::{issue_credit_note, IssueCreditNoteRequest};
use ar_rs::write_offs::{write_off_invoice, WriteOffInvoiceRequest};
use common::{generate_test_tenant, get_ar_pool};
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
    issue_credit_note(pool, req).await.map_err(|e| anyhow::anyhow!("{}", e))?;
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
    write_off_invoice(pool, req).await.map_err(|e| anyhow::anyhow!("{}", e))?;
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

    println!("✅ Credit note reduces aging: {} minor units remaining", snapshot.days_1_30_minor);

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
    assert_eq!(snapshot.invoice_count, 1, "Only one open invoice should remain");

    println!("✅ Write-off removes invoice from aging, {} minor units remaining", snapshot.total_outstanding_minor);

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

    println!("✅ Partial credit note: {} minor units remaining in 61-90 bucket", snapshot.days_61_90_minor);

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

    println!("✅ Payment + credit note: {} minor units remaining", snapshot.days_1_30_minor);

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

    println!("✅ Multiple credit notes accumulate: {} minor units remaining", snapshot.days_over_90_minor);

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
    assert_eq!(snapshot.current_minor, 3000, "Second invoice in current bucket");
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
    assert_eq!(snapshot.current_minor, 8000, "Invoice A: 10000 - 2000 = 8000 current");
    // Invoice B: 8000 - 3000 payment - 1000 credit = 4000 in days_1_30
    assert_eq!(snapshot.days_1_30_minor, 4000, "Invoice B: 8000 - 3000 - 1000 = 4000 days_1_30");
    // Invoice C: written off, should not appear
    assert_eq!(snapshot.days_31_60_minor, 0, "Invoice C: fully written off, no balance");
    // Invoice D: 3000 in days_61_90
    assert_eq!(snapshot.days_61_90_minor, 3000, "Invoice D: 3000 days_61_90");
    assert_eq!(snapshot.days_over_90_minor, 0, "No invoices over 90 days");

    // Total = 8000 + 4000 + 3000 = 15000
    assert_eq!(snapshot.total_outstanding_minor, 15000, "Total: 8000 + 4000 + 3000 = 15000");
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
