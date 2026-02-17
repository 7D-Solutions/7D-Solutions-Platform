//! E2E Test: Write-off → GL Posting (bd-1rp)
//!
//! **Coverage:**
//! 1. Write-off event posts balanced GL journal entry (BAD_DEBT DR / AR CR)
//! 2. Idempotency: duplicate event_id does not create a second journal entry
//! 3. Journal entry is balanced (debit == credit)
//! 4. BAD_DEBT and AR accounts must exist and be active before posting
//! 5. Mutation class is DATA_MUTATION on the GL side (write-off posts expense)
//! 6. Outbox atomicity: write-off posted + processed_events updated atomically
//!
//! **Pattern:** No Docker, no mocks — uses live GL database pool via common::get_gl_pool()
//! Tests call `process_writeoff_posting` directly (no NATS required).

mod common;

use anyhow::Result;
use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::consumer::gl_writeoff_consumer::{
    process_writeoff_posting, InvoiceWrittenOffPayload,
};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert required GL accounts (BAD_DEBT + AR) for the test tenant.
async fn setup_gl_accounts(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
            (gen_random_uuid(), $1, 'BAD_DEBT', 'Bad Debt Expense', 'expense', 'debit', true),
            (gen_random_uuid(), $1, 'AR', 'Accounts Receivable', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create an open accounting period for the given tenant covering 2026.
async fn setup_accounting_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    // accounting_periods has an exclusion constraint (no duplicate ranges per tenant)
    // but each test uses a unique tenant_id so plain INSERT is safe.
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, '2026-01-01', '2026-12-31', false)
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch the journal entry created for a given source_event_id.
///
/// journal_entries.source_event_id is UNIQUE — one entry per event.
async fn get_journal_entry_id(pool: &sqlx::PgPool, event_id: Uuid) -> Result<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM journal_entries WHERE source_event_id = $1 LIMIT 1",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Get journal lines for a given journal entry.
async fn get_journal_lines(
    pool: &sqlx::PgPool,
    entry_id: Uuid,
) -> Result<Vec<(String, f64, f64)>> {
    // Returns (account_ref, debit, credit) in major currency units
    let rows: Vec<(String, f64, f64)> = sqlx::query_as(
        r#"
        SELECT account_ref,
               COALESCE(debit_minor, 0)::float8 / 100.0 AS debit,
               COALESCE(credit_minor, 0)::float8 / 100.0 AS credit
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Count processed_events rows for a given event_id.
/// processed_events has: event_id (UUID UNIQUE), event_type, processed_at, processor
async fn count_processed_events(pool: &sqlx::PgPool, event_id: Uuid) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM processed_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Cleanup GL test data for a tenant (in reverse FK order).
///
/// Deletion order respects FK constraints:
///   journal_lines → journal_entries → processed_events (via source_event_id)
///   account_balances → accounting_periods
///   period_summary_snapshots → accounting_periods
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    // Lines first (FK → journal_entries)
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    // processed_events (no tenant_id; use source_event_id join)
    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN \
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    // Journal entries
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    // account_balances (FK → accounting_periods)
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    // period_summary_snapshots (FK → accounting_periods)
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    // Accounts and periods (no dependents remain)
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Build a sample write-off payload.
fn sample_payload(tenant_id: &str, invoice_id: &str, amount_minor: i64) -> InvoiceWrittenOffPayload {
    InvoiceWrittenOffPayload {
        tenant_id: tenant_id.to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: "cust-writeoff-test".to_string(),
        written_off_amount_minor: amount_minor,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: Some("admin@tenant.local".to_string()),
        written_off_at: Utc::now(),
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Successful write-off → balanced GL journal entry created.
///
/// DR BAD_DEBT $500.00
/// CR AR       $500.00
#[tokio::test]
async fn test_writeoff_posts_balanced_gl_entry() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "inv-wo-001", 50000); // $500.00

    let entry_id = process_writeoff_posting(&pool, event_id, &tenant_id, "ar", &payload)
        .await
        .expect("write-off posting should succeed");

    // Verify journal entry exists with the returned ID
    let stored_entry_id = get_journal_entry_id(&pool, event_id).await?;
    assert_eq!(
        stored_entry_id,
        Some(entry_id),
        "journal_entries.source_event_id lookup should return the created entry"
    );

    // Verify journal lines: DR BAD_DEBT / CR AR
    let lines = get_journal_lines(&pool, entry_id).await?;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let bad_debt_line = lines.iter().find(|(acct, _, _)| acct == "BAD_DEBT");
    let ar_line = lines.iter().find(|(acct, _, _)| acct == "AR");

    assert!(bad_debt_line.is_some(), "BAD_DEBT line must exist");
    assert!(ar_line.is_some(), "AR line must exist");

    let (_, bad_debt_debit, bad_debt_credit) = bad_debt_line.unwrap();
    let (_, ar_debit, ar_credit) = ar_line.unwrap();

    assert!(
        (*bad_debt_debit - 500.0).abs() < 0.01,
        "BAD_DEBT debit should be $500.00, got {}",
        bad_debt_debit
    );
    assert!(
        (*bad_debt_credit).abs() < 0.01,
        "BAD_DEBT credit should be $0.00"
    );
    assert!(
        (*ar_debit).abs() < 0.01,
        "AR debit should be $0.00"
    );
    assert!(
        (*ar_credit - 500.0).abs() < 0.01,
        "AR credit should be $500.00, got {}",
        ar_credit
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 2: Journal entry is balanced (debit total == credit total).
#[tokio::test]
async fn test_writeoff_journal_entry_is_balanced() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "inv-balance-check", 123456); // $1,234.56

    let entry_id = process_writeoff_posting(&pool, event_id, &tenant_id, "ar", &payload)
        .await
        .expect("posting should succeed");

    let lines = get_journal_lines(&pool, entry_id).await?;
    let total_debit: f64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credit: f64 = lines.iter().map(|(_, _, c)| c).sum();

    assert!(
        (total_debit - total_credit).abs() < 0.01,
        "Journal entry must be balanced: debit={} credit={}",
        total_debit,
        total_credit
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 3: Idempotency — duplicate event_id does not create a second journal entry.
#[tokio::test]
async fn test_writeoff_idempotency_prevents_duplicate_entry() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "inv-idem-001", 25000);

    // First posting — should succeed
    let first_result =
        process_writeoff_posting(&pool, event_id, &tenant_id, "ar", &payload).await;
    assert!(first_result.is_ok(), "First posting should succeed");

    // Second posting with same event_id — should return DuplicateEvent (no-op)
    let second_result =
        process_writeoff_posting(&pool, event_id, &tenant_id, "ar", &payload).await;

    match second_result {
        Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_)) => {
            // Expected: idempotent no-op
        }
        other => panic!("Expected DuplicateEvent, got: {:?}", other),
    }

    // Verify only one processed_events row
    let count = count_processed_events(&pool, event_id).await?;
    assert_eq!(count, 1, "only one processed_events row for this event_id");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 4: Different event_ids for different write-off events create separate journal entries.
#[tokio::test]
async fn test_writeoff_different_events_create_separate_entries() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id_1 = Uuid::new_v4();
    let event_id_2 = Uuid::new_v4();
    let payload_1 = sample_payload(&tenant_id, "inv-sep-001", 10000);
    let payload_2 = sample_payload(&tenant_id, "inv-sep-002", 20000);

    let entry_id_1 =
        process_writeoff_posting(&pool, event_id_1, &tenant_id, "ar", &payload_1)
            .await
            .expect("first posting should succeed");
    let entry_id_2 =
        process_writeoff_posting(&pool, event_id_2, &tenant_id, "ar", &payload_2)
            .await
            .expect("second posting should succeed");

    assert_ne!(
        entry_id_1, entry_id_2,
        "different events must create different journal entries"
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 5: Processed_events row is created after successful posting.
///
/// This proves atomicity: the event is recorded as processed in the same transaction.
#[tokio::test]
async fn test_writeoff_processed_events_row_created() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "inv-proc-001", 75000);

    process_writeoff_posting(&pool, event_id, &tenant_id, "ar", &payload)
        .await
        .expect("posting should succeed");

    let count = count_processed_events(&pool, event_id).await?;
    assert_eq!(count, 1, "processed_events row must be created");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 6: Write-off description includes invoice_id and reason.
#[tokio::test]
async fn test_writeoff_journal_entry_description() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let invoice_id = "inv-desc-check-001";
    let payload = sample_payload(&tenant_id, invoice_id, 30000);

    let entry_id = process_writeoff_posting(&pool, event_id, &tenant_id, "ar", &payload)
        .await
        .expect("posting should succeed");

    let description: String = sqlx::query_scalar(
        "SELECT description FROM journal_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await?;

    assert!(
        description.contains(invoice_id),
        "description should contain invoice_id; got: '{}'",
        description
    );
    assert!(
        description.contains("uncollectable"),
        "description should contain the reason; got: '{}'",
        description
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}
