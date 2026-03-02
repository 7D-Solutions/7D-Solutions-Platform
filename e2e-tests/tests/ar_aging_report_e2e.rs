//! E2E Test: AR Aging Report — invoices bucketed by 0-30, 31-60, 61-90, 90+ days overdue (bd-28dx)
//!
//! Validates the AR aging projection correctly categorises invoices by days overdue:
//!   1. Four overdue invoices land in the correct bucket each
//!   2. Bucket totals are mutually exclusive and collectively exhaustive
//!   3. Tenant isolation — aging only returns invoices for the target tenant

mod common;

use anyhow::Result;
use ar_rs::aging::refresh_aging;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;

// ============================================================================
// Helpers
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
    .bind(format!("aging-report-{}@test.local", tenant_id))
    .bind(format!("Aging Report Test {}", tenant_id))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert an open invoice whose due_at is `days_ago` days in the past.
async fn make_overdue_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    days_ago: u32,
) -> Result<i32> {
    let invoice_id: i32 = sqlx::query_scalar(&format!(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'usd', NOW() - INTERVAL '{days_ago} days', NOW(), NOW())
        RETURNING id
        "#,
        days_ago = days_ago
    ))
    .bind(tenant_id)
    .bind(format!("inv_{}", uuid::Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;
    Ok(invoice_id)
}

// ============================================================================
// Test 1: All four overdue buckets — correct placement and totals sum
// ============================================================================

/// Creates four overdue invoices, one per aging bucket, then verifies:
///   - Each invoice lands in its expected bucket
///   - Bucket totals are mutually exclusive (no double-counting)
///   - Grand total equals the sum of all individual bucket amounts
#[tokio::test]
#[serial]
async fn test_aging_four_buckets_correct_placement() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

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

    // Invoice A: 10 days overdue → days_1_30 bucket
    make_overdue_invoice(&ar_pool, &tenant_id, customer_id, 10_000, 10).await?;
    // Invoice B: 45 days overdue → days_31_60 bucket
    make_overdue_invoice(&ar_pool, &tenant_id, customer_id, 20_000, 45).await?;
    // Invoice C: 75 days overdue → days_61_90 bucket
    make_overdue_invoice(&ar_pool, &tenant_id, customer_id, 30_000, 75).await?;
    // Invoice D: 100 days overdue → days_over_90 bucket
    make_overdue_invoice(&ar_pool, &tenant_id, customer_id, 40_000, 100).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Each invoice lands in the right bucket
    assert_eq!(
        snapshot.current_minor, 0,
        "No current (not-yet-due) invoices expected"
    );
    assert_eq!(
        snapshot.days_1_30_minor, 10_000,
        "Invoice A (10 days overdue) must be in 0-30 bucket"
    );
    assert_eq!(
        snapshot.days_31_60_minor, 20_000,
        "Invoice B (45 days overdue) must be in 31-60 bucket"
    );
    assert_eq!(
        snapshot.days_61_90_minor, 30_000,
        "Invoice C (75 days overdue) must be in 61-90 bucket"
    );
    assert_eq!(
        snapshot.days_over_90_minor, 40_000,
        "Invoice D (100 days overdue) must be in 90+ bucket"
    );

    // Grand total equals the sum of all buckets (mutually exclusive + exhaustive)
    let expected_total = 10_000 + 20_000 + 30_000 + 40_000;
    assert_eq!(
        snapshot.total_outstanding_minor, expected_total,
        "Total outstanding must equal sum of all bucket amounts"
    );

    let bucket_sum = snapshot.current_minor
        + snapshot.days_1_30_minor
        + snapshot.days_31_60_minor
        + snapshot.days_61_90_minor
        + snapshot.days_over_90_minor;
    assert_eq!(
        bucket_sum, snapshot.total_outstanding_minor,
        "Bucket amounts must sum to total_outstanding (no double-counting, no gaps)"
    );

    assert_eq!(
        snapshot.invoice_count, 4,
        "All four invoices must be counted"
    );

    println!(
        "✅ All four buckets correct: 0-30={}, 31-60={}, 61-90={}, 90+={}; total={}",
        snapshot.days_1_30_minor,
        snapshot.days_31_60_minor,
        snapshot.days_61_90_minor,
        snapshot.days_over_90_minor,
        snapshot.total_outstanding_minor,
    );

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

// ============================================================================
// Test 2: Tenant isolation — aging does not bleed across tenants
// ============================================================================

/// Creates invoices for two independent tenants and verifies the aging
/// projection for each tenant only contains its own data.
#[tokio::test]
#[serial]
async fn test_aging_tenant_isolation() -> Result<()> {
    let tenant_a = generate_test_tenant();
    let tenant_b = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    // Clean both tenants upfront
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_a,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_b,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Tenant A: one overdue invoice (50 days)
    let customer_a = make_customer(&ar_pool, &tenant_a).await?;
    make_overdue_invoice(&ar_pool, &tenant_a, customer_a, 99_000, 50).await?;

    // Tenant B: one overdue invoice (20 days, different amount)
    let customer_b = make_customer(&ar_pool, &tenant_b).await?;
    make_overdue_invoice(&ar_pool, &tenant_b, customer_b, 55_000, 20).await?;

    // Compute aging for each tenant independently
    let snapshot_a = refresh_aging(&ar_pool, &tenant_a, customer_a).await?;
    let snapshot_b = refresh_aging(&ar_pool, &tenant_b, customer_b).await?;

    // Tenant A: 50 days → days_31_60 bucket
    assert_eq!(
        snapshot_a.days_31_60_minor, 99_000,
        "Tenant A: 50-day overdue invoice must appear in 31-60 bucket"
    );
    assert_eq!(
        snapshot_a.days_1_30_minor, 0,
        "Tenant A: must not contain tenant B's 20-day invoice"
    );
    assert_eq!(
        snapshot_a.total_outstanding_minor, 99_000,
        "Tenant A total must only include its own invoice"
    );

    // Tenant B: 20 days → days_1_30 bucket
    assert_eq!(
        snapshot_b.days_1_30_minor, 55_000,
        "Tenant B: 20-day overdue invoice must appear in 0-30 bucket"
    );
    assert_eq!(
        snapshot_b.days_31_60_minor, 0,
        "Tenant B: must not contain tenant A's 50-day invoice"
    );
    assert_eq!(
        snapshot_b.total_outstanding_minor, 55_000,
        "Tenant B total must only include its own invoice"
    );

    // Double-check: Snapshot app_id must match the queried tenant
    assert_eq!(
        snapshot_a.app_id, tenant_a,
        "Snapshot app_id must match the queried tenant"
    );
    assert_eq!(
        snapshot_b.app_id, tenant_b,
        "Snapshot app_id must match the queried tenant"
    );

    println!(
        "✅ Tenant isolation confirmed: tenant_a total={}, tenant_b total={}",
        snapshot_a.total_outstanding_minor, snapshot_b.total_outstanding_minor,
    );

    // Cleanup both tenants
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_a,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_b,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}
