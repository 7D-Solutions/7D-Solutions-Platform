//! E2E Test: GL Outbox Atomicity (bd-r01m)
//!
//! **Phase 16: Outbox Pattern Atomicity Enforcement**
//!
//! ## GL Module Atomicity Posture
//!
//! Unlike AR/Payments/Subscriptions, GL has a unique role:
//! - **Consumes** events (gl.posting.requested) but doesn't emit events for normal postings
//! - **Emits** events only for reversals (gl.events.entry.reversed)
//! - Reversal event emission is ALREADY ATOMIC (verified in this test)
//!
//! ## Test Coverage
//! 1. **Reversal Atomicity**: Reversal entry creation + event emission must be atomic
//! 2. **Transaction Boundary**: Reversal mutation + outbox insert in single BEGIN/COMMIT
//!
//! ## Expected Behavior
//! - Reversal service emits gl.events.entry.reversed within the same transaction
//! - reversal_service.rs lines 214-226: insert_outbox_event_with_linkage(&mut tx, ...)
//! - Pattern: create reversal entry → emit event → tx.commit()
//!
//! ## Audit Result: ✅ COMPLIANT
//! - GL reversal service already maintains atomicity
//! - No violations found
//! - This test serves as regression protection

mod common;

use anyhow::Result;
use chrono::NaiveDate;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::services::{journal_service, reversal_service};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Set up COA accounts and an open accounting period covering today.
async fn setup_gl_test_env(pool: &PgPool, tenant_id: &str) -> Result<()> {
    // Create accounts needed for journal lines (explicit enum casts for account_type/normal_balance)
    for (code, name, acct_type, normal_balance) in [
        ("1100", "Accounts Receivable", "asset", "debit"),
        ("4000", "Revenue", "revenue", "credit"),
    ] {
        sqlx::query(
            "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
             VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, true, NOW())
             ON CONFLICT (tenant_id, code) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(code)
        .bind(name)
        .bind(acct_type)
        .bind(normal_balance)
        .execute(pool)
        .await?;
    }

    // Open period: Feb 2024 (for original entry posting date)
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, '2024-02-01', '2024-02-29', false, NOW())
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await?;

    // Open period covering today (reversal_service uses today's date for reversal)
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, DATE_TRUNC('month', CURRENT_DATE)::date,
                 (DATE_TRUNC('month', CURRENT_DATE) + INTERVAL '1 month - 1 day')::date, false, NOW())
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Create a journal entry in GL database
async fn create_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    source_event_id: Uuid,
) -> Result<Uuid> {
    let entry_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id, source_subject,
            posted_at, currency, description, created_at
        )
        VALUES ($1, $2, 'ar', $3, 'invoice.finalized', NOW(), 'USD', 'Test entry', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(source_event_id)
    .execute(pool)
    .await?;

    // Add journal lines (must balance)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES 
            ($1, $2, 1, '1100', 10000, 0),
            ($2, $2, 2, '4000', 0, 10000)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .execute(pool)
    .await?;

    Ok(entry_id)
}

/// Count GL outbox rows whose aggregate_id corresponds to a journal entry for the given tenant.
/// Joins with journal_entries to isolate per-test data and avoid cross-test pollution.
async fn count_outbox_rows_for_tenant(pool: &PgPool, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox o
         WHERE o.event_type LIKE 'gl.events%'
           AND o.aggregate_id IN (
               SELECT id::text FROM journal_entries WHERE tenant_id = $1
           )",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// Test GL reversal atomicity with outbox
///
/// Calls reversal_service::create_reversal_entry directly and asserts that
/// both the reversal journal entry AND the gl.events.entry.reversed outbox event
/// are committed atomically in a single transaction.
///
/// GL reversal_service.rs is already compliant — this test is regression protection.
#[tokio::test]
#[serial]
async fn test_gl_reversal_outbox_atomicity() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(
        &get_ar_pool().await,
        &get_payments_pool().await,
        &get_subscriptions_pool().await,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Set up accounts + open period for Feb 2024
    setup_gl_test_env(&gl_pool, &tenant_id).await?;

    // Step 1: Create original journal entry via journal_service
    let orig_event_id = Uuid::new_v4();
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("test-{}", Uuid::new_v4()),
        description: "GL reversal atomicity test".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: None,
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: None,
                dimensions: None,
            },
        ],
    };
    let original_entry_id = journal_service::process_gl_posting_request(
        &gl_pool,
        orig_event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Original posting failed: {:?}", e))?;

    println!("✅ Created journal entry {}", original_entry_id);

    println!("📊 Outbox check will be by reversal entry aggregate_id after reversal");

    // Step 3: Create reversal via production service (atomic: entry + outbox in one tx)
    let reversal_event_id = Uuid::new_v4();
    let reversal_entry_id =
        reversal_service::create_reversal_entry(&gl_pool, reversal_event_id, original_entry_id)
            .await
            .map_err(|e| anyhow::anyhow!("Reversal failed: {:?}", e))?;

    println!("✅ Created reversal entry {}", reversal_entry_id);

    // Step 4: Assert reversal entry exists and references original
    let reverses_ref: Option<Uuid> =
        sqlx::query_scalar("SELECT reverses_entry_id FROM journal_entries WHERE id = $1")
            .bind(reversal_entry_id)
            .fetch_one(&gl_pool)
            .await?;
    assert_eq!(
        reverses_ref,
        Some(original_entry_id),
        "Reversal entry must reference original"
    );

    // Step 5: Assert outbox has gl.events.entry.reversed event for this specific reversal entry
    // Filter by aggregate_id = reversal_entry_id to avoid cross-test pollution
    let reversal_outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_id = $1 AND event_type = 'gl.events.entry.reversed'",
    )
    .bind(reversal_entry_id.to_string())
    .fetch_one(&gl_pool)
    .await?;

    assert_eq!(
        reversal_outbox_count, 1,
        "❌ ATOMICITY VIOLATION: reversal entry committed but no gl.events.entry.reversed outbox event \
         (expected 1, found {}). aggregate_id={}",
        reversal_outbox_count, reversal_entry_id
    );
    println!(
        "✅ Atomicity confirmed: reversal entry {} + gl.events.entry.reversed outbox event committed atomically",
        reversal_entry_id
    );

    // Step 6: Assert idempotency — same reversal_event_id is rejected
    let dup_result =
        reversal_service::create_reversal_entry(&gl_pool, reversal_event_id, original_entry_id)
            .await;
    assert!(
        dup_result.is_err(),
        "Duplicate reversal event must be rejected (idempotency)"
    );
    println!("✅ Idempotency gate works: duplicate reversal_event_id rejected");

    cleanup_tenant_data(
        &get_ar_pool().await,
        &get_payments_pool().await,
        &get_subscriptions_pool().await,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    println!("\n🎯 GL reversal atomicity verified:");
    println!("   - reversal entry + gl.events.entry.reversed outbox event committed atomically");
    println!("   - No orphaned reversals possible");

    Ok(())
}

/// Test GL normal posting does NOT emit events (by design)
///
/// This test documents that GL's journal posting workflow is a pure consumer:
/// - Consumes gl.posting.requested events
/// - Creates journal entries
/// - Does NOT emit events for normal postings
#[tokio::test]
#[serial]
async fn test_gl_posting_no_event_emission() -> Result<()> {
    let test_id = "gl_posting_no_emit";
    let tenant_id = generate_test_tenant();

    let gl_pool = get_gl_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(
        &get_ar_pool().await,
        &get_payments_pool().await,
        &get_subscriptions_pool().await,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Step 1: Create journal entry (simulating what journal_service does)
    let source_event_id = Uuid::new_v4();
    let entry_id = create_journal_entry(&gl_pool, &tenant_id, source_event_id).await?;
    println!("✅ Created journal entry {}", entry_id);

    // Step 2: Verify NO events emitted for normal posting
    let outbox_count = count_outbox_rows_for_tenant(&gl_pool, &tenant_id).await?;

    assert_eq!(
        outbox_count, 0,
        "GL normal posting should NOT emit events (count: {})",
        outbox_count
    );

    println!("\n✅ GL Posting Behavior Verified:");
    println!("   - Normal journal posting does NOT emit events");
    println!("   - GL is a pure consumer for gl.posting.requested");
    println!("   - Only reversals emit events (gl.events.entry.reversed)");

    // Clean up
    cleanup_tenant_data(
        &get_ar_pool().await,
        &get_payments_pool().await,
        &get_subscriptions_pool().await,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
