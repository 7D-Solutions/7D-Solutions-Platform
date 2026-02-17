/// Phase 10 Governance E2E Tests (bd-1oe rewrite — no Docker)
///
/// Tests GL governance: COA validation, period enforcement, and reversal logic.
/// All tests call GL service functions in-process against the shared database.
///
/// Strategy:
/// - Use get_gl_pool() from common (shared DB connection)
/// - Call gl_rs::services functions directly — no Docker, no NATS
/// - Setup accounts/periods directly in DB; clean up via unique tenant_id
/// - Remove #[ignore] — all tests run in default cargo test sweep

mod common;

use anyhow::Result;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::services::{journal_service, reversal_service};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test Setup Helpers
// ============================================================================

/// Insert test accounts into the GL accounts table for this tenant.
///
/// Creates:
/// - 1100: Cash (asset, active)
/// - 1200: Accounts Receivable (asset, active)
/// - 4000: Revenue (revenue, active)
/// - 9999: Inactive Account (expense, INACTIVE)
async fn setup_test_accounts(pool: &PgPool, tenant_id: &str) -> Result<()> {
    let accounts = [
        (Uuid::new_v4(), "1100", "Cash", "asset", "debit", true),
        (Uuid::new_v4(), "1200", "Accounts Receivable", "asset", "debit", true),
        (Uuid::new_v4(), "4000", "Revenue", "revenue", "credit", true),
        (Uuid::new_v4(), "9999", "Inactive Account", "expense", "debit", false),
    ];

    for (id, code, name, acct_type, normal_balance, is_active) in accounts {
        sqlx::query(
            r#"
            INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
            VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, $7, NOW())
            ON CONFLICT (tenant_id, code) DO NOTHING
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(code)
        .bind(name)
        .bind(acct_type)
        .bind(normal_balance)
        .bind(is_active)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Insert a closed period (Jan 2024) and an open period (Feb 2024) for this tenant.
async fn setup_test_periods(pool: &PgPool, tenant_id: &str) -> Result<()> {
    // Closed period: January 2024
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, closed_at, created_at)
        VALUES ($1, $2, '2024-01-01', '2024-01-31', true, NOW(), NOW())
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await?;

    // Open period: February 2024
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-02-01', '2024-02-29', false, NOW())
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Build a minimal GlPostingRequestV1 with the given lines.
fn make_posting_request(posting_date: &str, lines: Vec<(&str, f64, f64)>) -> GlPostingRequestV1 {
    GlPostingRequestV1 {
        posting_date: posting_date.to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("test-{}", Uuid::new_v4()),
        description: "Phase 10 governance test".to_string(),
        lines: lines
            .into_iter()
            .map(|(account_ref, debit, credit)| JournalLine {
                account_ref: account_ref.to_string(),
                debit,
                credit,
                memo: None,
                dimensions: None,
            })
            .collect(),
    }
}

// ============================================================================
// Test: Invalid Account Rejection
// ============================================================================

/// Posting with a non-existent account_ref is rejected before any DB write.
#[tokio::test]
#[serial]
async fn test_phase10_invalid_account_rejection() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;

    // Setup valid accounts — but NOT "8888"
    setup_test_accounts(&pool, &tenant_id).await?;
    setup_test_periods(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = make_posting_request(
        "2024-02-15",
        vec![
            ("1200", 100.0, 0.0), // valid AR account
            ("8888", 0.0, 100.0), // does NOT exist
        ],
    );

    let result = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "Posting with invalid account_ref should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("8888") || err_msg.contains("not found") || err_msg.contains("Validation"),
        "Error should mention invalid account: {}",
        err_msg
    );

    // No journal entry created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(&tenant_id)
    .bind(event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 0, "No journal entry should exist for rejected posting");

    println!("✅ Invalid account correctly rejected");
    Ok(())
}

// ============================================================================
// Test: Inactive Account Rejection
// ============================================================================

/// Posting with an inactive account (9999) is rejected by COA validation.
#[tokio::test]
#[serial]
async fn test_phase10_inactive_account_rejection() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;

    // Setup accounts — 9999 is inactive
    setup_test_accounts(&pool, &tenant_id).await?;
    setup_test_periods(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = make_posting_request(
        "2024-02-15",
        vec![
            ("1200", 100.0, 0.0), // valid, active
            ("9999", 0.0, 100.0), // inactive
        ],
    );

    let result = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "Posting with inactive account should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("9999") || err_msg.contains("inactive") || err_msg.contains("Validation"),
        "Error should mention inactive account: {}",
        err_msg
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(&tenant_id)
    .bind(event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 0, "No journal entry should exist for rejected posting");

    println!("✅ Inactive account correctly rejected");
    Ok(())
}

// ============================================================================
// Test: Closed Period Rejection
// ============================================================================

/// Posting to a date in a closed period (January 2024) is rejected.
#[tokio::test]
#[serial]
async fn test_phase10_closed_period_rejection() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;

    setup_test_accounts(&pool, &tenant_id).await?;
    setup_test_periods(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    // January 2024 is the closed period
    let payload = make_posting_request(
        "2024-01-15",
        vec![
            ("1200", 100.0, 0.0),
            ("4000", 0.0, 100.0),
        ],
    );

    let result = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "Posting to closed period should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("closed") || err_msg.contains("Period"),
        "Error should mention closed period: {}",
        err_msg
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(&tenant_id)
    .bind(event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 0, "No journal entry should exist for rejected posting");

    println!("✅ Closed period posting correctly rejected");
    Ok(())
}

// ============================================================================
// Test: Valid Posting in Open Period
// ============================================================================

/// A valid posting to an open period (February 2024) creates a journal entry.
#[tokio::test]
#[serial]
async fn test_phase10_valid_posting_open_period() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;

    setup_test_accounts(&pool, &tenant_id).await?;
    setup_test_periods(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    // February 2024 is open
    let payload = make_posting_request(
        "2024-02-15",
        vec![
            ("1200", 100.0, 0.0),
            ("4000", 0.0, 100.0),
        ],
    );

    let entry_id = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Posting failed: {}", e))?;

    // Verify journal entry was created
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE id = $1 AND tenant_id = $2",
    )
    .bind(entry_id)
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(entry_count, 1, "Journal entry should be created");

    // Verify two journal lines exist
    let line_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(line_count, 2, "Two journal lines should exist");

    println!("✅ Valid posting created journal entry {} with 2 lines", entry_id);
    Ok(())
}

// ============================================================================
// Test: Valid Reversal Creation
// ============================================================================

/// A reversal of an existing journal entry creates an inverse entry.
#[tokio::test]
#[serial]
async fn test_phase10_valid_reversal() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;

    setup_test_accounts(&pool, &tenant_id).await?;
    setup_test_periods(&pool, &tenant_id).await?;

    // Step 1: Create original journal entry
    let orig_event_id = Uuid::new_v4();
    let payload = make_posting_request(
        "2024-02-15",
        vec![
            ("1200", 100.0, 0.0),
            ("4000", 0.0, 100.0),
        ],
    );
    let original_entry_id = journal_service::process_gl_posting_request(
        &pool,
        orig_event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Original posting failed: {}", e))?;

    println!("✅ Original entry created: {}", original_entry_id);

    // Step 2: Create reversal
    let reversal_event_id = Uuid::new_v4();
    let reversal_entry_id = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id,
        original_entry_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Reversal failed: {}", e))?;

    println!("✅ Reversal entry created: {}", reversal_entry_id);

    // Verify reversal references original
    let reverses_ref: Option<Uuid> = sqlx::query_scalar(
        "SELECT reverses_entry_id FROM journal_entries WHERE id = $1",
    )
    .bind(reversal_entry_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        reverses_ref,
        Some(original_entry_id),
        "Reversal entry must reference original entry"
    );

    // Verify lines are inverted
    let orig_lines: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT debit_minor, credit_minor FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(original_entry_id)
    .fetch_all(&pool)
    .await?;

    let rev_lines: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT debit_minor, credit_minor FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(reversal_entry_id)
    .fetch_all(&pool)
    .await?;

    assert_eq!(orig_lines.len(), rev_lines.len(), "Line counts must match");
    for ((orig_d, orig_c), (rev_d, rev_c)) in orig_lines.iter().zip(rev_lines.iter()) {
        assert_eq!(orig_d, rev_c, "Original debit should equal reversal credit");
        assert_eq!(orig_c, rev_d, "Original credit should equal reversal debit");
    }

    println!("✅ Reversal lines are correctly inverted");
    Ok(())
}

// ============================================================================
// Test: Idempotent Reversal (same reversal event processed twice)
// ============================================================================

/// Replaying the same reversal_event_id is idempotent — only one reversal is created.
#[tokio::test]
#[serial]
async fn test_phase10_idempotent_reversal() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;

    setup_test_accounts(&pool, &tenant_id).await?;
    setup_test_periods(&pool, &tenant_id).await?;

    // Create original entry
    let orig_event_id = Uuid::new_v4();
    let payload = make_posting_request(
        "2024-02-15",
        vec![
            ("1200", 100.0, 0.0),
            ("4000", 0.0, 100.0),
        ],
    );
    let original_entry_id = journal_service::process_gl_posting_request(
        &pool,
        orig_event_id,
        &tenant_id,
        "test",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Original posting failed: {}", e))?;

    // First reversal — succeeds
    let reversal_event_id = Uuid::new_v4();
    let reversal_id = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id,
        original_entry_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!("First reversal failed: {}", e))?;

    println!("✅ First reversal created: {}", reversal_id);

    // Replay same reversal_event_id — must return DuplicateEvent, not create a second reversal
    let second_result = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id, // same event_id
        original_entry_id,
    )
    .await;

    assert!(
        second_result.is_err(),
        "Replaying same reversal_event_id must return an error (DuplicateEvent)"
    );
    let err_msg = second_result.unwrap_err().to_string();
    assert!(
        err_msg.contains("duplicate") || err_msg.contains("already"),
        "Error should indicate duplicate event: {}",
        err_msg
    );

    // Exactly one reversal entry in DB
    let reversal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE reverses_entry_id = $1",
    )
    .bind(original_entry_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        reversal_count, 1,
        "Exactly one reversal should exist even after duplicate event replay"
    );

    println!("✅ Idempotent reversal: duplicate event rejected, only 1 reversal in DB");
    Ok(())
}
