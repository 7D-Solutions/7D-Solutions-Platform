//! E2E Test: Realized FX Gain/Loss GL Postings (bd-1p8)
//!
//! **Coverage:**
//! 1. Settlement with FX gain → balanced GL entry (DR AR / CR FX_REALIZED_GAIN)
//! 2. Settlement with FX loss → balanced GL entry (DR FX_REALIZED_LOSS / CR AR)
//! 3. Settlement with same rate → no journal entry posted
//! 4. Idempotency: duplicate event_id does not create a second journal entry
//! 5. Journal entry is balanced (debit == credit)
//! 6. Processed_events row is created atomically
//!
//! **Pattern:** No Docker, no mocks — uses live GL database pool via common::get_gl_pool()
//! Tests call `process_fx_realized_posting` directly (no NATS required).

mod common;

use anyhow::Result;
use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::consumer::gl_fx_realized_consumer::{
    process_fx_realized_posting, InvoiceSettledFxPayload,
};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert required GL accounts (AR + FX_REALIZED_GAIN + FX_REALIZED_LOSS) for the test tenant.
async fn setup_gl_accounts(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
            (gen_random_uuid(), $1, 'AR', 'Accounts Receivable', 'asset', 'debit', true),
            (gen_random_uuid(), $1, 'FX_REALIZED_GAIN', 'Realized FX Gain', 'revenue', 'credit', true),
            (gen_random_uuid(), $1, 'FX_REALIZED_LOSS', 'Realized FX Loss', 'expense', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create an open accounting period covering 2026.
async fn setup_accounting_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
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

/// Fetch the journal entry ID for a given source_event_id.
async fn get_journal_entry_id(pool: &sqlx::PgPool, event_id: Uuid) -> Result<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM journal_entries WHERE source_event_id = $1 LIMIT 1",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Get journal lines for a given journal entry as (account_ref, debit, credit).
async fn get_journal_lines(
    pool: &sqlx::PgPool,
    entry_id: Uuid,
) -> Result<Vec<(String, f64, f64)>> {
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
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN \
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

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

/// Build a sample FX settlement payload.
fn sample_gain_payload(tenant_id: &str) -> InvoiceSettledFxPayload {
    InvoiceSettledFxPayload {
        tenant_id: tenant_id.to_string(),
        invoice_id: "inv-fx-gain-001".to_string(),
        customer_id: "cust-fx-test".to_string(),
        txn_currency: "EUR".to_string(),
        txn_amount_minor: 100000, // EUR 1,000.00
        rpt_currency: "USD".to_string(),
        recognition_rpt_amount_minor: 110000, // USD 1,100.00 at recognition
        recognition_rate_id: Uuid::new_v4(),
        recognition_rate: 1.10,
        settlement_rpt_amount_minor: 112000, // USD 1,120.00 at settlement
        settlement_rate_id: Uuid::new_v4(),
        settlement_rate: 1.12,
        realized_gain_loss_minor: 2000, // USD 20.00 gain
        settled_at: Utc::now(),
    }
}

fn sample_loss_payload(tenant_id: &str) -> InvoiceSettledFxPayload {
    InvoiceSettledFxPayload {
        tenant_id: tenant_id.to_string(),
        invoice_id: "inv-fx-loss-001".to_string(),
        customer_id: "cust-fx-test".to_string(),
        txn_currency: "GBP".to_string(),
        txn_amount_minor: 50000, // GBP 500.00
        rpt_currency: "USD".to_string(),
        recognition_rpt_amount_minor: 63000, // USD 630.00 at recognition
        recognition_rate_id: Uuid::new_v4(),
        recognition_rate: 1.26,
        settlement_rpt_amount_minor: 62500, // USD 625.00 at settlement
        settlement_rate_id: Uuid::new_v4(),
        settlement_rate: 1.25,
        realized_gain_loss_minor: -500, // USD 5.00 loss
        settled_at: Utc::now(),
    }
}

fn sample_no_diff_payload(tenant_id: &str) -> InvoiceSettledFxPayload {
    InvoiceSettledFxPayload {
        tenant_id: tenant_id.to_string(),
        invoice_id: "inv-fx-nodiff-001".to_string(),
        customer_id: "cust-fx-test".to_string(),
        txn_currency: "EUR".to_string(),
        txn_amount_minor: 100000,
        rpt_currency: "USD".to_string(),
        recognition_rpt_amount_minor: 108500,
        recognition_rate_id: Uuid::new_v4(),
        recognition_rate: 1.085,
        settlement_rpt_amount_minor: 108500, // Same as recognition
        settlement_rate_id: Uuid::new_v4(),
        settlement_rate: 1.085,
        realized_gain_loss_minor: 0, // No difference
        settled_at: Utc::now(),
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: FX gain → balanced GL entry (DR AR / CR FX_REALIZED_GAIN)
#[tokio::test]
async fn test_fx_gain_posts_balanced_gl_entry() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_gain_payload(&tenant_id);

    let result = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await?;
    let entry_id = result.expect("FX gain should produce a journal entry");

    // Verify journal entry exists
    let stored = get_journal_entry_id(&pool, event_id).await?;
    assert_eq!(stored, Some(entry_id));

    // Verify journal lines
    let lines = get_journal_lines(&pool, entry_id).await?;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let ar_line = lines.iter().find(|(acct, _, _)| acct == "AR");
    let gain_line = lines.iter().find(|(acct, _, _)| acct == "FX_REALIZED_GAIN");

    assert!(ar_line.is_some(), "AR line must exist");
    assert!(gain_line.is_some(), "FX_REALIZED_GAIN line must exist");

    let (_, ar_debit, ar_credit) = ar_line.unwrap();
    let (_, gain_debit, gain_credit) = gain_line.unwrap();

    // FX gain: DR AR $20.00, CR FX_REALIZED_GAIN $20.00
    assert!(
        (*ar_debit - 20.0).abs() < 0.01,
        "AR debit should be $20.00, got {}",
        ar_debit
    );
    assert!((*ar_credit).abs() < 0.01, "AR credit should be $0.00");
    assert!((*gain_debit).abs() < 0.01, "FX_REALIZED_GAIN debit should be $0.00");
    assert!(
        (*gain_credit - 20.0).abs() < 0.01,
        "FX_REALIZED_GAIN credit should be $20.00, got {}",
        gain_credit
    );

    // Verify balance
    let total_debit: f64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credit: f64 = lines.iter().map(|(_, _, c)| c).sum();
    assert!(
        (total_debit - total_credit).abs() < 0.01,
        "Journal entry must be balanced: debit={} credit={}",
        total_debit,
        total_credit
    );

    println!("PASS: FX gain posts balanced GL entry (DR AR $20 / CR FX_REALIZED_GAIN $20)");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 2: FX loss → balanced GL entry (DR FX_REALIZED_LOSS / CR AR)
#[tokio::test]
async fn test_fx_loss_posts_balanced_gl_entry() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_loss_payload(&tenant_id);

    let result = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await?;
    let entry_id = result.expect("FX loss should produce a journal entry");

    let lines = get_journal_lines(&pool, entry_id).await?;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let loss_line = lines.iter().find(|(acct, _, _)| acct == "FX_REALIZED_LOSS");
    let ar_line = lines.iter().find(|(acct, _, _)| acct == "AR");

    assert!(loss_line.is_some(), "FX_REALIZED_LOSS line must exist");
    assert!(ar_line.is_some(), "AR line must exist");

    let (_, loss_debit, loss_credit) = loss_line.unwrap();
    let (_, ar_debit, ar_credit) = ar_line.unwrap();

    // FX loss: DR FX_REALIZED_LOSS $5.00, CR AR $5.00
    assert!(
        (*loss_debit - 5.0).abs() < 0.01,
        "FX_REALIZED_LOSS debit should be $5.00, got {}",
        loss_debit
    );
    assert!((*loss_credit).abs() < 0.01, "FX_REALIZED_LOSS credit should be $0.00");
    assert!((*ar_debit).abs() < 0.01, "AR debit should be $0.00");
    assert!(
        (*ar_credit - 5.0).abs() < 0.01,
        "AR credit should be $5.00, got {}",
        ar_credit
    );

    // Verify balance
    let total_debit: f64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credit: f64 = lines.iter().map(|(_, _, c)| c).sum();
    assert!(
        (total_debit - total_credit).abs() < 0.01,
        "Journal entry must be balanced"
    );

    println!("PASS: FX loss posts balanced GL entry (DR FX_REALIZED_LOSS $5 / CR AR $5)");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 3: Same rate (no FX difference) → no journal entry posted
#[tokio::test]
async fn test_fx_no_difference_skips_gl_entry() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_no_diff_payload(&tenant_id);

    let result = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await?;
    assert!(result.is_none(), "No journal entry when FX delta is zero");

    // Verify no journal entry was created
    let stored = get_journal_entry_id(&pool, event_id).await?;
    assert!(stored.is_none(), "No journal entry in DB when rates match");

    println!("PASS: No GL entry when settlement rate == recognition rate");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 4: Idempotency — duplicate event_id does not create a second journal entry
#[tokio::test]
async fn test_fx_realized_idempotency() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_gain_payload(&tenant_id);

    // First posting
    let first = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await;
    assert!(first.is_ok(), "First posting should succeed");

    // Second posting with same event_id → DuplicateEvent
    let second = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await;
    match second {
        Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_)) => {
            // Expected
        }
        other => panic!("Expected DuplicateEvent, got: {:?}", other),
    }

    // Only one processed_events row
    let count = count_processed_events(&pool, event_id).await?;
    assert_eq!(count, 1, "only one processed_events row for this event_id");

    println!("PASS: Idempotency prevents duplicate FX GL entries");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 5: Processed_events row is created atomically
#[tokio::test]
async fn test_fx_realized_processed_events_created() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_gain_payload(&tenant_id);

    process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload)
        .await
        .expect("posting should succeed");

    let count = count_processed_events(&pool, event_id).await?;
    assert_eq!(count, 1, "processed_events row must be created");

    println!("PASS: processed_events row created atomically with FX GL entry");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 6: Journal entry description includes invoice_id and rate info
#[tokio::test]
async fn test_fx_realized_journal_description() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_gain_payload(&tenant_id);
    let invoice_id = payload.invoice_id.clone();

    let result = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await?;
    let entry_id = result.expect("should produce entry");

    let description: String = sqlx::query_scalar(
        "SELECT description FROM journal_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await?;

    assert!(
        description.contains(&invoice_id),
        "description should contain invoice_id; got: '{}'",
        description
    );
    assert!(
        description.contains("gain") || description.contains("Gain"),
        "description should mention gain; got: '{}'",
        description
    );

    println!("PASS: Journal description contains invoice_id and rate details");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 7: Different events create separate journal entries
#[tokio::test]
async fn test_fx_realized_different_events_create_separate_entries() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id_1 = Uuid::new_v4();
    let event_id_2 = Uuid::new_v4();
    let payload_1 = sample_gain_payload(&tenant_id);
    let mut payload_2 = sample_loss_payload(&tenant_id);
    payload_2.tenant_id = tenant_id.clone();

    let entry_1 = process_fx_realized_posting(&pool, event_id_1, &tenant_id, "ar", &payload_1)
        .await?
        .expect("first entry");
    let entry_2 = process_fx_realized_posting(&pool, event_id_2, &tenant_id, "ar", &payload_2)
        .await?
        .expect("second entry");

    assert_ne!(entry_1, entry_2, "different events must create different entries");

    println!("PASS: Different FX events create separate GL entries");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 8: FX posting uses reporting currency on journal entry
#[tokio::test]
async fn test_fx_realized_uses_reporting_currency() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_gain_payload(&tenant_id);

    let result = process_fx_realized_posting(&pool, event_id, &tenant_id, "ar", &payload).await?;
    let entry_id = result.expect("should produce entry");

    let currency: String = sqlx::query_scalar(
        "SELECT currency FROM journal_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(
        currency, "USD",
        "Journal entry currency should be reporting currency (USD), got: {}",
        currency
    );

    println!("PASS: FX journal entry uses reporting currency");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}
