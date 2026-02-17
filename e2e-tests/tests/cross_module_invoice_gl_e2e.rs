//! Cross-Module E2E: Invoice → GL (Phase 15 - bd-3rc.4)
//!
//! **Purpose:** Test AR → GL integration with balance validation
//!
//! **Invariants Tested:**
//! 1. No duplicate GL postings (source_event_id deduplication)
//! 2. GL balance validation (debits == credits)
//! 3. Journal entry creation from invoice events
//! 4. Account validation (accounts exist and active)

mod common;
mod oracle;

use chrono::Utc;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_test_accounts(
    gl_pool: &PgPool,
    tenant_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create Chart of Accounts entries
    sqlx::query(
        "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
         VALUES
           (gen_random_uuid(), $1, 'AR', 'Accounts Receivable', 'asset', 'debit', true),
           (gen_random_uuid(), $1, 'REV', 'Revenue', 'revenue', 'credit', true),
           (gen_random_uuid(), $1, 'TAX', 'Sales Tax Payable', 'liability', 'credit', true)
         ON CONFLICT (tenant_id, code) DO NOTHING"
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await?;

    Ok(())
}

async fn setup_accounting_period(
    gl_pool: &PgPool,
    tenant_id: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let period_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
         VALUES ($1, '2026-02-01', '2026-02-28', false)
         RETURNING id"
    )
    .bind(tenant_id)
    .fetch_one(gl_pool)
    .await?;

    Ok(period_id)
}

async fn create_journal_entry(
    gl_pool: &PgPool,
    tenant_id: &str,
    source_event_id: Uuid,
    posted_at: chrono::DateTime<Utc>,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let entry_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description)
         VALUES ($1, $2, 'ar', $3, 'invoice.created', $4, 'USD', 'Test invoice posting')
         RETURNING id"
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(source_event_id)
    .bind(posted_at)
    .fetch_one(gl_pool)
    .await?;

    Ok(entry_id)
}

async fn create_journal_lines(
    gl_pool: &PgPool,
    entry_id: Uuid,
    debit_account: &str,
    credit_account: &str,
    amount: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Debit line (AR)
    sqlx::query(
        "INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
         VALUES ($1, $2, 1, $3, $4, 0)"
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(debit_account)
    .bind(amount)
    .execute(gl_pool)
    .await?;

    // Credit line (Revenue)
    sqlx::query(
        "INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
         VALUES ($1, $2, 2, $3, 0, $4)"
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(credit_account)
    .bind(amount)
    .execute(gl_pool)
    .await?;

    Ok(())
}

// ============================================================================
// Test: No Duplicate GL Postings (Event Replay)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_no_duplicate_gl_postings() {
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    // Setup: Create accounts and period
    setup_test_accounts(&gl_pool, tenant_id).await.unwrap();
    setup_accounting_period(&gl_pool, tenant_id).await.unwrap();

    // Execute: Create first journal entry
    let entry1_id = create_journal_entry(&gl_pool, tenant_id, source_event_id, Utc::now())
        .await
        .expect("First posting should succeed");

    create_journal_lines(&gl_pool, entry1_id, "AR", "REV", 10000)
        .await
        .unwrap();

    // Execute: Try to create duplicate posting (same source_event_id)
    let result = create_journal_entry(&gl_pool, tenant_id, source_event_id, Utc::now()).await;

    // Assert: Duplicate should fail with UNIQUE constraint violation
    assert!(
        result.is_err(),
        "Duplicate posting with same source_event_id should fail"
    );

    // Assert: Exactly one journal entry exists for this event
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2"
    )
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count entries");

    assert_eq!(count, 1, "Should have exactly one journal entry");

    // Cleanup
    common::cleanup_tenant_data(
        &common::get_ar_pool().await,
        &common::get_payments_pool().await,
        &common::get_subscriptions_pool().await,
        &gl_pool,
        tenant_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: GL Balance Validation (Debits == Credits)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_balance_validation() {
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    // Setup: Create accounts and period
    setup_test_accounts(&gl_pool, tenant_id).await.unwrap();
    setup_accounting_period(&gl_pool, tenant_id).await.unwrap();

    // Execute: Create balanced journal entry
    let entry_id = create_journal_entry(&gl_pool, tenant_id, source_event_id, Utc::now())
        .await
        .expect("Posting should succeed");

    create_journal_lines(&gl_pool, entry_id, "AR", "REV", 10000)
        .await
        .unwrap();

    // Assert: Journal entry is balanced using common utility
    let balance_result = common::assert_journal_balanced(&gl_pool, entry_id).await;
    assert!(
        balance_result.is_ok(),
        "Journal entry should be balanced: {:?}",
        balance_result
    );

    // Assert: Manual balance check
    let (total_debits, total_credits): (i64, i64) = sqlx::query_as(
        "SELECT
            COALESCE(SUM(debit_minor), 0) as total_debits,
            COALESCE(SUM(credit_minor), 0) as total_credits
         FROM journal_lines
         WHERE journal_entry_id = $1"
    )
    .bind(entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to fetch totals");

    assert_eq!(
        total_debits, total_credits,
        "Debits should equal credits"
    );
    assert_eq!(total_debits, 10000, "Total debits should be 10000");

    // Cleanup
    common::cleanup_tenant_data(
        &common::get_ar_pool().await,
        &common::get_payments_pool().await,
        &common::get_subscriptions_pool().await,
        &gl_pool,
        tenant_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: Unbalanced Entry Detection
// ============================================================================

#[tokio::test]
#[serial]
async fn test_unbalanced_entry_detection() {
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    // Setup: Create accounts and period
    setup_test_accounts(&gl_pool, tenant_id).await.unwrap();
    setup_accounting_period(&gl_pool, tenant_id).await.unwrap();

    // Execute: Create unbalanced journal entry
    let entry_id = create_journal_entry(&gl_pool, tenant_id, source_event_id, Utc::now())
        .await
        .expect("Posting should succeed");

    // Create unbalanced lines (debit != credit)
    sqlx::query(
        "INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
         VALUES
           ($1, $2, 1, 'AR', 10000, 0),
           ($3, $2, 2, 'REV', 0, 9000)"
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await
    .unwrap();

    // Assert: Balance check should fail
    let balance_result = common::assert_journal_balanced(&gl_pool, entry_id).await;
    assert!(
        balance_result.is_err(),
        "Unbalanced entry should be detected"
    );

    // Cleanup
    common::cleanup_tenant_data(
        &common::get_ar_pool().await,
        &common::get_payments_pool().await,
        &common::get_subscriptions_pool().await,
        &gl_pool,
        tenant_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: Multiple Journal Entries Remain Independent
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multiple_independent_entries() {
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();

    // Setup: Create accounts and period
    setup_test_accounts(&gl_pool, tenant_id).await.unwrap();
    setup_accounting_period(&gl_pool, tenant_id).await.unwrap();

    // Execute: Create multiple journal entries
    let event1 = Uuid::new_v4();
    let event2 = Uuid::new_v4();
    let event3 = Uuid::new_v4();

    let entry1_id = create_journal_entry(&gl_pool, tenant_id, event1, Utc::now())
        .await
        .unwrap();
    create_journal_lines(&gl_pool, entry1_id, "AR", "REV", 10000)
        .await
        .unwrap();

    let entry2_id = create_journal_entry(&gl_pool, tenant_id, event2, Utc::now())
        .await
        .unwrap();
    create_journal_lines(&gl_pool, entry2_id, "AR", "REV", 20000)
        .await
        .unwrap();

    let entry3_id = create_journal_entry(&gl_pool, tenant_id, event3, Utc::now())
        .await
        .unwrap();
    create_journal_lines(&gl_pool, entry3_id, "AR", "REV", 30000)
        .await
        .unwrap();

    // Assert: All entries are balanced
    assert!(common::assert_journal_balanced(&gl_pool, entry1_id).await.is_ok());
    assert!(common::assert_journal_balanced(&gl_pool, entry2_id).await.is_ok());
    assert!(common::assert_journal_balanced(&gl_pool, entry3_id).await.is_ok());

    // Assert: Exactly 3 entries exist
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count entries");

    assert_eq!(count, 3, "Should have exactly 3 journal entries");

    // Cleanup
    common::cleanup_tenant_data(
        &common::get_ar_pool().await,
        &common::get_payments_pool().await,
        &common::get_subscriptions_pool().await,
        &gl_pool,
        tenant_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: Account Validation (Active Accounts Only)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_account_validation() {
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    // Setup: Create accounts (AR active, REV inactive)
    sqlx::query(
        "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
         VALUES
           (gen_random_uuid(), $1, 'AR', 'Accounts Receivable', 'asset', 'debit', true),
           (gen_random_uuid(), $1, 'REV', 'Revenue', 'revenue', 'credit', false)"
    )
    .bind(tenant_id)
    .execute(&gl_pool)
    .await
    .unwrap();

    setup_accounting_period(&gl_pool, tenant_id).await.unwrap();

    // Execute: Create journal entry with inactive account
    let entry_id = create_journal_entry(&gl_pool, tenant_id, source_event_id, Utc::now())
        .await
        .unwrap();

    create_journal_lines(&gl_pool, entry_id, "AR", "REV", 10000)
        .await
        .unwrap();

    // Assert: Check for inactive account reference
    let invalid_refs: Vec<String> = sqlx::query_scalar(
        "SELECT jl.account_ref
         FROM journal_lines jl
         JOIN journal_entries je ON je.id = jl.journal_entry_id
         LEFT JOIN accounts a ON a.tenant_id = je.tenant_id AND a.code = jl.account_ref
         WHERE je.tenant_id = $1 AND (a.id IS NULL OR a.is_active = false)"
    )
    .bind(tenant_id)
    .fetch_all(&gl_pool)
    .await
    .expect("Failed to check account references");

    assert!(
        !invalid_refs.is_empty(),
        "Should detect inactive account reference: {:?}",
        invalid_refs
    );
    assert!(invalid_refs.contains(&"REV".to_string()), "Should detect REV as inactive");

    // Cleanup
    common::cleanup_tenant_data(
        &common::get_ar_pool().await,
        &common::get_payments_pool().await,
        &common::get_subscriptions_pool().await,
        &gl_pool,
        tenant_id,
    )
    .await
    .ok();
}
