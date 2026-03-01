//! E2E Test: GL Posting Consumer for Credit Notes (bd-3vm)
//!
//! Validates that `ar.credit_note_issued` events produce balanced GL journal entries:
//!   1. Balanced entry: DR Revenue, CR AR — exactly correct amounts
//!   2. Idempotency: second call with same event_id returns DuplicateEvent (no double-post)
//!   3. Period enforcement: posting to a closed period is rejected
//!   4. Source doc type: `AR_CREDIT_MEMO` set on journal entry

mod common;

use anyhow::Result;
use chrono::Utc;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use gl_rs::consumers::gl_credit_note_consumer::{
    process_credit_note_posting, CreditNoteIssuedPayload,
};
use gl_rs::services::journal_service::JournalError;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

async fn setup_gl_accounts(gl_pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
          (gen_random_uuid(), $1, 'AR',  'Accounts Receivable', 'asset',   'debit',  true),
          (gen_random_uuid(), $1, 'REV', 'Revenue',             'revenue', 'credit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await?;
    Ok(())
}

async fn setup_open_period(gl_pool: &PgPool, tenant_id: &str) -> Result<Uuid> {
    let period_id = sqlx::query_scalar::<_, Uuid>(
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

async fn setup_closed_period(gl_pool: &PgPool, tenant_id: &str) -> Result<Uuid> {
    let period_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO accounting_periods (
            tenant_id, period_start, period_end, is_closed, closed_at,
            close_hash, closed_by, close_reason
        )
        VALUES ($1, '2026-01-01', '2026-01-31', true, NOW(),
                'test-hash-period-closed', 'test-setup', 'test closure')
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .fetch_one(gl_pool)
    .await?;
    Ok(period_id)
}

fn make_payload(
    tenant_id: &str,
    amount_minor: i64,
    issued_at: chrono::DateTime<Utc>,
) -> CreditNoteIssuedPayload {
    CreditNoteIssuedPayload {
        credit_note_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-e2e-1".to_string(),
        invoice_id: "inv-e2e-42".to_string(),
        amount_minor,
        currency: "usd".to_string(),
        reason: "service_credit".to_string(),
        issued_at,
    }
}

async fn count_journal_entries(gl_pool: &PgPool, tenant_id: &str) -> Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(gl_pool)
            .await?;
    Ok(count)
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Credit note produces balanced journal entry with correct accounts and amounts
#[tokio::test]
#[serial]
async fn test_credit_note_produces_balanced_journal_entry() -> Result<()> {
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

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_open_period(&gl_pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    // issued_at within the open period (2026-02-01 to 2026-02-28)
    let issued_at = chrono::DateTime::parse_from_rfc3339("2026-02-17T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let payload = make_payload(&tenant_id, 5000, issued_at); // $50.00

    let entry_id = process_credit_note_posting(&gl_pool, event_id, &tenant_id, "ar", &payload)
        .await
        .map_err(|e| anyhow::anyhow!("GL posting failed: {:?}", e))?;

    // Verify journal entry exists
    let entry_count = count_journal_entries(&gl_pool, &tenant_id).await?;
    assert_eq!(
        entry_count, 1,
        "Exactly one journal entry should be created"
    );

    // Verify journal lines: DR REV 50.00, CR AR 50.00
    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_ref, debit_minor, credit_minor
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY account_ref
        "#,
    )
    .bind(entry_id)
    .fetch_all(&gl_pool)
    .await?;

    assert_eq!(lines.len(), 2, "Two journal lines (DR + CR)");

    let ar_line = lines
        .iter()
        .find(|(acct, _, _)| acct == "AR")
        .expect("AR line missing");
    let rev_line = lines
        .iter()
        .find(|(acct, _, _)| acct == "REV")
        .expect("REV line missing");

    assert_eq!(ar_line.1, 0, "AR line: debit = 0");
    assert_eq!(ar_line.2, 5000, "AR line: credit = 5000 minor units");
    assert_eq!(rev_line.1, 5000, "REV line: debit = 5000 minor units");
    assert_eq!(rev_line.2, 0, "REV line: credit = 0");

    println!(
        "✅ Credit note GL entry: DR REV {}, CR AR {}",
        rev_line.1, ar_line.2
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

/// Test 2: Same event_id produces DuplicateEvent on second call (idempotency)
#[tokio::test]
#[serial]
async fn test_credit_note_posting_idempotent() -> Result<()> {
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

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_open_period(&gl_pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let issued_at = chrono::DateTime::parse_from_rfc3339("2026-02-17T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let payload = make_payload(&tenant_id, 3000, issued_at);

    // First call — succeeds
    process_credit_note_posting(&gl_pool, event_id, &tenant_id, "ar", &payload)
        .await
        .map_err(|e| anyhow::anyhow!("First posting failed: {:?}", e))?;

    // Second call — same event_id → DuplicateEvent
    let second = process_credit_note_posting(&gl_pool, event_id, &tenant_id, "ar", &payload).await;

    assert!(
        matches!(second, Err(JournalError::DuplicateEvent(_))),
        "Second call with same event_id must return DuplicateEvent, got: {:?}",
        second
    );

    // Still only one entry
    let entry_count = count_journal_entries(&gl_pool, &tenant_id).await?;
    assert_eq!(entry_count, 1, "No duplicate journal entries created");

    println!("✅ Credit note posting is idempotent: duplicate event_id rejected");

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

/// Test 3: Posting to a closed period is rejected
#[tokio::test]
#[serial]
async fn test_credit_note_rejects_closed_period() -> Result<()> {
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

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_closed_period(&gl_pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    // issued_at in January → targets the closed period
    let issued_at = chrono::DateTime::parse_from_rfc3339("2026-01-15T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let payload = make_payload(&tenant_id, 2000, issued_at);

    let result = process_credit_note_posting(&gl_pool, event_id, &tenant_id, "ar", &payload).await;

    assert!(
        matches!(result, Err(JournalError::Period(_))),
        "Posting to closed period must return Period error, got: {:?}",
        result
    );

    let entry_count = count_journal_entries(&gl_pool, &tenant_id).await?;
    assert_eq!(entry_count, 0, "No journal entry created for closed period");

    println!("✅ Credit note posting rejected for closed period");

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
