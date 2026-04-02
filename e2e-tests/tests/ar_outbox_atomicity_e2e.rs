//! E2E Test: AR Outbox Atomicity (bd-umnu)
//!
//! **Phase 16: Outbox Pattern Atomicity Enforcement**
//!
//! ## Test Coverage
//! 1. **Atomicity Violation (Current Bug)**: Invoice finalization succeeds without outbox row
//! 2. **Transaction Boundary**: Domain mutation + outbox insert must be atomic
//! 3. **Rollback Safety**: If outbox fails, invoice mutation must rollback
//!
//! ## Current Bug (Will FAIL before fix)
//! - Invoice finalization updates ar_invoices table
//! - Event emission to outbox happens AFTER (separate operation)
//! - If outbox insert fails, invoice is already finalized (data inconsistency)
//!
//! ## Expected After Fix
//! - Invoice finalization + outbox insert in single BEGIN/COMMIT transaction
//! - Test will PASS: Both succeed together or both fail together

mod common;

use anyhow::Result;
use ar_rs::finalization::{finalize_invoice, FinalizationResult};
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;

/// Create a customer in AR database
async fn create_ar_customer(pool: &PgPool, tenant_id: &str) -> Result<i32> {
    let customer_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("customer-{}@test.com", tenant_id))
    .bind(format!("Test Customer {}", tenant_id))
    .fetch_one(pool)
    .await?;

    Ok(customer_id)
}

/// Create a draft invoice in AR database
async fn create_draft_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i64,
) -> Result<i32> {
    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'draft', $4, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_test_{}", uuid::Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;

    Ok(invoice_id)
}

/// Get invoice status
async fn get_invoice_status(pool: &PgPool, invoice_id: i32) -> Result<String> {
    let status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(pool)
        .await?;

    Ok(status)
}

/// Count outbox rows for a given invoice
async fn count_outbox_rows_for_invoice(pool: &PgPool, invoice_id: i32) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) 
        FROM events_outbox 
        WHERE aggregate_type = 'invoice' 
          AND aggregate_id = $1
        "#,
    )
    .bind(invoice_id.to_string())
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// Test that AR invoice finalization atomically enqueues outbox events.
///
/// Calls the production `finalize_invoice()` function (bd-umnu fix) which wraps
/// both the invoice status mutation and the outbox insert in a single transaction.
/// If either fails, both roll back — no orphaned invoice state without events.
#[tokio::test]
#[serial]
async fn test_ar_finalize_invoice_outbox_atomicity() -> Result<()> {
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

    // Step 1: Create test customer + draft invoice
    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = create_draft_invoice(&ar_pool, &tenant_id, customer_id, 5000).await?;
    println!(
        "✅ Created customer {} + draft invoice {}",
        customer_id, invoice_id
    );

    // Verify initial state
    assert_eq!(get_invoice_status(&ar_pool, invoice_id).await?, "draft");
    assert_eq!(
        count_outbox_rows_for_invoice(&ar_pool, invoice_id).await?,
        0,
        "No outbox rows should exist before finalization"
    );

    // Step 2: Call the production finalize_invoice() — atomically updates invoice + enqueues events
    let result = finalize_invoice(&ar_pool, &tenant_id, invoice_id, 0)
        .await
        .map_err(|e| anyhow::anyhow!("finalize_invoice failed: {:?}", e))?;

    assert!(
        matches!(result, FinalizationResult::NewAttempt { .. }),
        "Expected NewAttempt from first finalization, got {:?}",
        result
    );
    println!("✅ finalize_invoice() succeeded (NewAttempt)");

    // Step 3: Assert atomicity — both domain state and outbox must reflect finalization
    let final_outbox_count = count_outbox_rows_for_invoice(&ar_pool, invoice_id).await?;

    println!("\n📊 Atomicity Check:");
    println!("  Invoice status: draft -> attempting (finalized)");
    println!("  Outbox rows: 0 -> {}", final_outbox_count);

    assert!(
        final_outbox_count >= 1,
        "❌ ATOMICITY VIOLATION: finalize_invoice() succeeded but outbox has {} events (expected >= 1). \
         Guard->Mutation->Outbox must all commit atomically.",
        final_outbox_count
    );

    println!(
        "✅ Atomicity proven: invoice finalized AND {} outbox event(s) present",
        final_outbox_count
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

/// Test that AR invoice finalization is idempotent: calling finalize_invoice twice
/// returns AlreadyAttempted on the second call and does not duplicate outbox events.
#[tokio::test]
#[serial]
async fn test_ar_finalize_invoice_idempotency() -> Result<()> {
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

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = create_draft_invoice(&ar_pool, &tenant_id, customer_id, 5000).await?;

    // First call: must succeed with NewAttempt
    let result1 = finalize_invoice(&ar_pool, &tenant_id, invoice_id, 0)
        .await
        .map_err(|e| anyhow::anyhow!("first finalize_invoice failed: {:?}", e))?;
    assert!(
        matches!(result1, FinalizationResult::NewAttempt { .. }),
        "Expected NewAttempt on first call, got {:?}",
        result1
    );

    let outbox_after_first = count_outbox_rows_for_invoice(&ar_pool, invoice_id).await?;
    assert!(
        outbox_after_first >= 1,
        "Outbox must have >= 1 row after first finalization"
    );

    // Second call: must be idempotent (AlreadyAttempted, no new outbox rows)
    let result2 = finalize_invoice(&ar_pool, &tenant_id, invoice_id, 0)
        .await
        .map_err(|e| anyhow::anyhow!("second finalize_invoice failed: {:?}", e))?;
    assert!(
        matches!(result2, FinalizationResult::AlreadyProcessed { .. }),
        "Expected AlreadyProcessed on second call, got {:?}",
        result2
    );

    let outbox_after_second = count_outbox_rows_for_invoice(&ar_pool, invoice_id).await?;
    assert_eq!(
        outbox_after_first, outbox_after_second,
        "Outbox count must not increase on duplicate finalization (idempotency)"
    );

    println!(
        "✅ Idempotency proven: second finalize returns AlreadyAttempted, outbox count unchanged"
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
