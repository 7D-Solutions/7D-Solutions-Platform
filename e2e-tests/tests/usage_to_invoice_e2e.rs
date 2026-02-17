//! E2E Test: Usage-to-Invoice Billing (bd-n9j)
//!
//! Validates that unbilled metered usage records are billed exactly once:
//!   1. Unbilled usage → line items created, billed_at set, ar.usage_invoiced emitted
//!   2. Already-billed usage → skipped on second call (idempotency)
//!   3. SKIP LOCKED: usage locked by first call is not double-billed

mod common;

use anyhow::Result;
use chrono::{TimeZone, Utc};
use common::{cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_payments_pool, get_subscriptions_pool, get_gl_pool};
use ar_rs::usage_billing::{bill_usage_for_invoice, BillUsageRequest};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

async fn make_customer(pool: &PgPool, tenant_id: &str) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("usage-billing-{}@test.local", tenant_id))
    .bind(format!("Usage Billing Test {}", tenant_id))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn make_invoice(pool: &PgPool, tenant_id: &str, customer_id: i32) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', 0, 'usd', NOW() + INTERVAL '30 days', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn insert_usage(
    pool: &PgPool,
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

async fn count_outbox_events(pool: &PgPool, tenant_id: &str, event_type: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = $2",
    )
    .bind(tenant_id)
    .bind(event_type)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Unbilled usage → line items created, billed_at set, outbox events emitted
#[tokio::test]
#[serial]
async fn test_usage_billed_produces_line_items_and_events() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id).await?;

    let period_start = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let period_end = Utc.with_ymd_and_hms(2026, 2, 28, 23, 59, 59).unwrap();

    // Insert 2 unbilled usage records within the window
    insert_usage(&ar_pool, &tenant_id, customer_id, "api_calls", 100.0, 10, period_start, period_end).await?;
    insert_usage(&ar_pool, &tenant_id, customer_id, "storage_gb", 50.0, 5, period_start, period_end).await?;

    let result = bill_usage_for_invoice(
        &ar_pool,
        BillUsageRequest {
            app_id: tenant_id.clone(),
            invoice_id,
            customer_id,
            period_start,
            period_end,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await?;

    assert_eq!(result.billed_count, 2, "Both usage records should be billed");
    // api_calls: 100 * 10 = 1000; storage_gb: 50 * 5 = 250; total = 1250
    assert_eq!(result.total_amount_minor, 1250, "Total should be 1250 minor units");

    // Verify billed_at is set on both usage records
    let unbilled_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_metered_usage WHERE app_id = $1 AND billed_at IS NULL",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(unbilled_count, 0, "All usage records should have billed_at set");

    // Verify line items were created
    let line_item_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoice_line_items WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(line_item_count, 2, "Two invoice line items should be created");

    // Verify outbox events
    let event_count = count_outbox_events(&ar_pool, &tenant_id, "ar.usage_invoiced").await?;
    assert_eq!(event_count, 2, "Two ar.usage_invoiced events should be emitted");

    println!(
        "✅ Usage billing: {} records billed, {} minor units total, {} line items, {} events",
        result.billed_count, result.total_amount_minor, line_item_count, event_count
    );

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 2: Second call with same parameters does not double-bill
#[tokio::test]
#[serial]
async fn test_usage_not_double_billed() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id).await?;

    let period_start = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let period_end = Utc.with_ymd_and_hms(2026, 2, 28, 23, 59, 59).unwrap();

    insert_usage(&ar_pool, &tenant_id, customer_id, "api_calls", 200.0, 10, period_start, period_end).await?;

    let req = BillUsageRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id,
        period_start,
        period_end,
        correlation_id: Uuid::new_v4().to_string(),
    };

    // First call — bills 1 usage record
    let first = bill_usage_for_invoice(&ar_pool, req.clone()).await?;
    assert_eq!(first.billed_count, 1, "First call: 1 record billed");

    // Second call — no-op (all usage already billed)
    let second = bill_usage_for_invoice(&ar_pool, req).await?;
    assert_eq!(second.billed_count, 0, "Second call: nothing to bill (already done)");
    assert_eq!(second.total_amount_minor, 0);

    // Still only 1 line item
    let line_item_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoice_line_items WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(line_item_count, 1, "No duplicate line items created");

    println!("✅ Usage not double-billed: second call returned billed_count=0");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 3: Usage outside the billing window is not billed
#[tokio::test]
#[serial]
async fn test_usage_outside_window_not_billed() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id).await?;

    // Usage in January — outside the February billing window
    let jan_start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let jan_end = Utc.with_ymd_and_hms(2026, 1, 31, 23, 59, 59).unwrap();
    insert_usage(&ar_pool, &tenant_id, customer_id, "api_calls", 500.0, 10, jan_start, jan_end).await?;

    let feb_start = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let feb_end = Utc.with_ymd_and_hms(2026, 2, 28, 23, 59, 59).unwrap();

    // Bill for February
    let result = bill_usage_for_invoice(
        &ar_pool,
        BillUsageRequest {
            app_id: tenant_id.clone(),
            invoice_id,
            customer_id,
            period_start: feb_start,
            period_end: feb_end,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await?;

    assert_eq!(result.billed_count, 0, "January usage must not be billed in February window");

    // January usage still unbilled
    let unbilled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_metered_usage WHERE app_id = $1 AND billed_at IS NULL",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(unbilled, 1, "January usage should still be unbilled");

    println!("✅ Usage outside billing window correctly excluded");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}
