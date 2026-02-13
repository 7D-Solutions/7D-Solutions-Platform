//! E2E Test: Reversal → Balances → Trial Balance (With Idempotency)
//!
//! This test validates the full path from journal reversal through balance updates
//! to trial balance reporting, with idempotency guarantees.
//!
//! Test Flow:
//! 1. Set up Chart of Accounts and accounting period
//! 2. Post initial journal entry
//! 3. Verify initial balances
//! 4. Publish reversal request event
//! 5. Wait for reversal consumer to process
//! 6. Verify balances are inverted correctly
//! 7. Verify trial balance reflects reversal
//! 8. Replay reversal request (idempotency)
//! 9. Verify balances remain unchanged after replay

use chrono::{Datelike, NaiveDate, Utc};
use event_bus::{EventBus, EventEnvelope, InMemoryBus};
use gl_rs::contracts::gl_entry_reverse_request_v1::GlEntryReverseRequestV1;
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::repos::{balance_repo, journal_repo};
use gl_rs::services::{journal_service, trial_balance_service};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

/// Setup test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

/// Helper to insert a test account into Chart of Accounts
async fn insert_test_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: AccountType,
    normal_balance: NormalBalance,
) -> Uuid {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .bind(true) // is_active
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test account");

    id
}

/// Helper to create a test accounting period
async fn insert_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    is_closed: bool,
) -> Uuid {
    let period_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .bind(is_closed)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test period");

    period_id
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    // Delete in correct order due to foreign key constraints
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup balances");

    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal lines");

    // Get event IDs for this tenant before deleting journal entries
    let event_ids: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT source_event_id FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .expect("Failed to fetch event IDs");

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal entries");

    // Delete processed events by event_id (processed_events doesn't have tenant_id)
    for event_id in event_ids {
        sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
            .bind(event_id)
            .execute(pool)
            .await
            .expect("Failed to cleanup processed event");
    }

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup accounts");

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup periods");
}

/// Helper to publish reversal request event to event bus
async fn publish_reversal_request(
    bus: Arc<dyn EventBus>,
    event_id: Uuid,
    tenant_id: &str,
    original_entry_id: Uuid,
) {
    let request = GlEntryReverseRequestV1 {
        original_entry_id,
        reason: Some("E2E test reversal".to_string()),
    };

    let envelope = EventEnvelope {
        event_id,
        occurred_at: Utc::now(),
        tenant_id: tenant_id.to_string(),
        source_module: "gl-e2e-test".to_string(),
        source_version: "0.1.0".to_string(),
        correlation_id: None,
        causation_id: None,
        payload: request,
    };

    let payload = serde_json::to_vec(&envelope).expect("Failed to serialize envelope");

    bus.publish("gl.events.entry.reverse.requested", payload)
        .await
        .expect("Failed to publish reversal request");
}

/// Wait for reversal to be processed by checking if reversal entry exists
async fn wait_for_reversal_processing(
    pool: &PgPool,
    original_entry_id: Uuid,
    timeout_secs: u64,
) -> Option<Uuid> {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    while start.elapsed() < timeout {
        // Check if a reversal entry exists (reverses_entry_id = original_entry_id)
        let reversal_entry_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM journal_entries WHERE reverses_entry_id = $1 LIMIT 1",
        )
        .bind(original_entry_id)
        .fetch_optional(pool)
        .await
        .expect("Failed to query reversal entry");

        if reversal_entry_id.is_some() {
            return reversal_entry_id;
        }

        sleep(Duration::from_millis(100)).await;
    }

    None
}

#[tokio::test]
#[serial]
async fn test_e2e_reversal_updates_balances_and_trial_balance() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-e2e-reversal-001";

    // Cleanup any leftover data from previous runs
    cleanup_test_data(&pool, tenant_id).await;

    // Setup: Create Chart of Accounts
    let _acct_ar = insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    let _acct_revenue = insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup: Create accounting period (current month to allow reversals)
    let now = Utc::now().date_naive();
    let period_start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1).unwrap();
    let period_end = if now.month() == 12 {
        NaiveDate::from_ymd_opt(now.year(), 12, 31).unwrap()
    } else {
        NaiveDate::from_ymd_opt(now.year(), now.month() + 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    };

    let period_id = insert_test_period(&pool, tenant_id, period_start, period_end, false).await;

    // Step 1: Post initial journal entry
    let event_id = Uuid::new_v4();
    let posting_request = GlPostingRequestV1 {
        posting_date: now.format("%Y-%m-%d").to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv-reversal-001".to_string(),
        description: "Test invoice for reversal E2E".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 1000.00,
                credit: 0.0,
                memo: Some("AR debit".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 1000.00,
                memo: Some("Revenue credit".to_string()),
                dimensions: None,
            },
        ],
    };

    let original_entry_id = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "gl.events.posting.requested",
        &posting_request,
    )
    .await
    .expect("Failed to process posting request");

    assert_ne!(original_entry_id, Uuid::nil(), "Entry ID should be generated");

    // Step 2: Verify initial balances
    let balance_ar_initial = balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
        .await
        .expect("Failed to query AR balance")
        .expect("AR balance should exist");

    assert_eq!(balance_ar_initial.debit_total_minor, 100000); // $1000.00
    assert_eq!(balance_ar_initial.credit_total_minor, 0);
    assert_eq!(balance_ar_initial.net_balance_minor, 100000);

    let balance_revenue_initial =
        balance_repo::find_by_grain(&pool, tenant_id, period_id, "4000", "USD")
            .await
            .expect("Failed to query revenue balance")
            .expect("Revenue balance should exist");

    assert_eq!(balance_revenue_initial.debit_total_minor, 0);
    assert_eq!(balance_revenue_initial.credit_total_minor, 100000); // $1000.00
    assert_eq!(balance_revenue_initial.net_balance_minor, -100000);

    // Step 3: Verify initial trial balance
    let trial_balance_initial =
        trial_balance_service::get_trial_balance(&pool, tenant_id, period_id, Some("USD"))
            .await
            .expect("Failed to get initial trial balance");

    assert_eq!(trial_balance_initial.rows.len(), 2);
    assert_eq!(trial_balance_initial.totals.total_debits, 100000);
    assert_eq!(trial_balance_initial.totals.total_credits, 100000);
    assert!(trial_balance_initial.totals.is_balanced);

    // Step 4: Initialize event bus and start reversal consumer
    // Use InMemoryBus for E2E tests (doesn't require external NATS)
    let bus = Arc::new(InMemoryBus::new()) as Arc<dyn EventBus>;

    // Start reversal consumer
    gl_rs::consumer::gl_reversal_consumer::start_gl_reversal_consumer(
        Arc::clone(&bus),
        pool.clone(),
    )
    .await;

    // Give consumer time to subscribe
    sleep(Duration::from_millis(500)).await;

    // Step 5: Publish reversal request
    let reversal_event_id = Uuid::new_v4();
    publish_reversal_request(
        Arc::clone(&bus),
        reversal_event_id,
        tenant_id,
        original_entry_id,
    )
    .await;

    // Step 6: Wait for reversal to be processed
    let reversal_entry_id = wait_for_reversal_processing(&pool, original_entry_id, 10)
        .await
        .expect("Reversal processing timed out");

    tracing::info!(
        original_entry_id = %original_entry_id,
        reversal_entry_id = %reversal_entry_id,
        "Reversal processed successfully"
    );

    // Step 7: Verify reversal entry exists and links back to original
    let (reversal_entry, reversal_lines) =
        journal_repo::fetch_entry_with_lines(&pool, reversal_entry_id)
            .await
            .expect("Failed to fetch reversal entry")
            .expect("Reversal entry should exist");

    assert_eq!(
        reversal_entry.reverses_entry_id,
        Some(original_entry_id),
        "Reversal entry should link back to original"
    );
    assert_eq!(reversal_lines.len(), 2, "Reversal should have 2 lines");

    // Verify lines are inverted
    let reversal_ar_line = reversal_lines
        .iter()
        .find(|l| l.account_ref == "1100")
        .expect("AR line should exist in reversal");
    assert_eq!(reversal_ar_line.debit_minor, 0, "AR debit should be inverted to 0");
    assert_eq!(reversal_ar_line.credit_minor, 100000, "AR credit should be 1000.00");

    let reversal_revenue_line = reversal_lines
        .iter()
        .find(|l| l.account_ref == "4000")
        .expect("Revenue line should exist in reversal");
    assert_eq!(reversal_revenue_line.debit_minor, 100000, "Revenue debit should be 1000.00");
    assert_eq!(reversal_revenue_line.credit_minor, 0, "Revenue credit should be inverted to 0");

    // Step 8: Verify balances are updated correctly after reversal
    let balance_ar_after = balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
        .await
        .expect("Failed to query AR balance after reversal")
        .expect("AR balance should exist after reversal");

    assert_eq!(
        balance_ar_after.debit_total_minor, 100000,
        "AR debit total should include both original and reversal"
    );
    assert_eq!(
        balance_ar_after.credit_total_minor, 100000,
        "AR credit total should include reversal"
    );
    assert_eq!(
        balance_ar_after.net_balance_minor, 0,
        "AR net balance should be zero after reversal"
    );

    let balance_revenue_after =
        balance_repo::find_by_grain(&pool, tenant_id, period_id, "4000", "USD")
            .await
            .expect("Failed to query revenue balance after reversal")
            .expect("Revenue balance should exist after reversal");

    assert_eq!(
        balance_revenue_after.debit_total_minor, 100000,
        "Revenue debit total should include reversal"
    );
    assert_eq!(
        balance_revenue_after.credit_total_minor, 100000,
        "Revenue credit total should include both original and reversal"
    );
    assert_eq!(
        balance_revenue_after.net_balance_minor, 0,
        "Revenue net balance should be zero after reversal"
    );

    // Step 9: Verify trial balance reflects reversal (net zero balances)
    let trial_balance_after =
        trial_balance_service::get_trial_balance(&pool, tenant_id, period_id, Some("USD"))
            .await
            .expect("Failed to get trial balance after reversal");

    assert_eq!(trial_balance_after.rows.len(), 2, "Should still have 2 accounts");
    assert_eq!(
        trial_balance_after.totals.total_debits, 200000,
        "Total debits should be 2000.00 (original + reversal)"
    );
    assert_eq!(
        trial_balance_after.totals.total_credits, 200000,
        "Total credits should be 2000.00 (original + reversal)"
    );
    assert!(trial_balance_after.totals.is_balanced, "Trial balance should still be balanced");

    // Verify individual account rows show net zero
    let ar_row = trial_balance_after
        .rows
        .iter()
        .find(|r| r.account_code == "1100")
        .expect("AR should be in trial balance");
    assert_eq!(ar_row.net_balance_minor, 0, "AR net should be zero");

    let revenue_row = trial_balance_after
        .rows
        .iter()
        .find(|r| r.account_code == "4000")
        .expect("Revenue should be in trial balance");
    assert_eq!(revenue_row.net_balance_minor, 0, "Revenue net should be zero");

    // Step 10: Test idempotency - replay the same reversal request
    tracing::info!("Testing idempotency by replaying reversal request");
    publish_reversal_request(
        Arc::clone(&bus),
        reversal_event_id, // Same event_id
        tenant_id,
        original_entry_id,
    )
    .await;

    // Wait a bit for processing
    sleep(Duration::from_secs(2)).await;

    // Step 11: Verify balances remain unchanged (idempotency)
    let balance_ar_idempotent =
        balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
            .await
            .expect("Failed to query AR balance after replay")
            .expect("AR balance should exist after replay");

    assert_eq!(
        balance_ar_idempotent.debit_total_minor, 100000,
        "AR debit should remain unchanged (idempotency)"
    );
    assert_eq!(
        balance_ar_idempotent.credit_total_minor, 100000,
        "AR credit should remain unchanged (idempotency)"
    );
    assert_eq!(
        balance_ar_idempotent.net_balance_minor, 0,
        "AR net should remain zero (idempotency)"
    );

    let balance_revenue_idempotent =
        balance_repo::find_by_grain(&pool, tenant_id, period_id, "4000", "USD")
            .await
            .expect("Failed to query revenue balance after replay")
            .expect("Revenue balance should exist after replay");

    assert_eq!(
        balance_revenue_idempotent.debit_total_minor, 100000,
        "Revenue debit should remain unchanged (idempotency)"
    );
    assert_eq!(
        balance_revenue_idempotent.credit_total_minor, 100000,
        "Revenue credit should remain unchanged (idempotency)"
    );
    assert_eq!(
        balance_revenue_idempotent.net_balance_minor, 0,
        "Revenue net should remain zero (idempotency)"
    );

    // Verify only ONE reversal entry exists (not duplicated)
    let reversal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE reverses_entry_id = $1",
    )
    .bind(original_entry_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to count reversal entries");

    assert_eq!(
        reversal_count, 1,
        "Should have exactly one reversal entry (idempotency)"
    );

    // Step 12: Verify trial balance remains unchanged after replay
    let trial_balance_idempotent =
        trial_balance_service::get_trial_balance(&pool, tenant_id, period_id, Some("USD"))
            .await
            .expect("Failed to get trial balance after replay");

    assert_eq!(trial_balance_idempotent.rows.len(), 2);
    assert_eq!(trial_balance_idempotent.totals.total_debits, 200000);
    assert_eq!(trial_balance_idempotent.totals.total_credits, 200000);
    assert!(trial_balance_idempotent.totals.is_balanced);

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    println!("✅ E2E Test Passed: Reversal updates balances correctly and trial balance matches");
    println!("✅ Idempotency Verified: Replaying reversal request does not change balances");
}
