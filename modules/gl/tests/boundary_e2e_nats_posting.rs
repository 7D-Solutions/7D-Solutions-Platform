//! Boundary E2E Test: NATS → Consumer → DB (Posting Path)
//!
//! This test validates the REAL ingress boundary for GL postings:
//! 1. Publishes actual NATS event to `gl.events.posting.requested`
//! 2. Waits for consumer to process
//! 3. Asserts journal entry created, balances updated
//! 4. Verifies replay safety (idempotency)
//!
//! ## Architecture Decision
//! Per ChatGPT guidance: "E2E for microservices means crossing the ACTUAL ingress boundary."
//! Write path ingress = NATS (not HTTP), so this test uses real NATS pub/sub.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - NATS at localhost:4222
//! - PostgreSQL at localhost:5438

mod common;

use chrono::{NaiveDate, Utc};
use common::get_test_pool;
use event_bus::{EventBus, EventEnvelope, NatsBus};
use gl_rs::contracts::gl_posting_request_v1::{
    Dimensions, GlPostingRequestV1, JournalLine, SourceDocType,
};
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::repos::balance_repo;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

/// Setup NATS event bus (requires NATS running on localhost:4222)
async fn setup_nats_bus() -> Arc<dyn EventBus> {
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

    let nats_client = async_nats::connect(&nats_url)
        .await
        .expect("Failed to connect to NATS - ensure docker compose is running");

    Arc::new(NatsBus::new(nats_client))
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
        ON CONFLICT (tenant_id, code) DO NOTHING
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

/// Helper to wait for event processing by polling processed_events table
async fn wait_for_event_processing(pool: &PgPool, event_id: Uuid, max_wait_secs: u64) -> bool {
    let start = std::time::Instant::now();

    while start.elapsed().as_secs() < max_wait_secs {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM processed_events WHERE event_id = $1)",
        )
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap_or(false);

        if exists {
            return true;
        }

        sleep(Duration::from_millis(100)).await;
    }

    false
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    // Delete in correct order due to foreign key constraints
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup balances");

    sqlx::query("DELETE FROM journal_lines WHERE entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal lines");

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal entries");

    sqlx::query("DELETE FROM processed_events WHERE event_id IN (SELECT event_id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok(); // May not exist, that's fine

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

#[tokio::test]
#[serial]
async fn test_boundary_nats_posting_creates_journal_and_balances() {
    // Setup
    let pool = get_test_pool().await;
    let bus = setup_nats_bus().await;
    let tenant_id = "tenant-boundary-nats-001";

    // Cleanup any leftover data
    cleanup_test_data(&pool, tenant_id).await;

    // Setup Chart of Accounts
    insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup accounting period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        false, // open
    )
    .await;

    // Create GL posting request payload
    let posting_request = GlPostingRequestV1 {
        posting_date: "2024-02-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv-nats-001".to_string(),
        description: "Boundary E2E test - NATS ingress".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 1500.00,
                credit: 0.0,
                memo: Some("AR debit".to_string()),
                dimensions: Some(Dimensions {
                    customer_id: Some("cust-001".to_string()),
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: None,
                    project: None,
                }),
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 1500.00,
                memo: Some("Revenue credit".to_string()),
                dimensions: None,
            },
        ],
    };

    // Create event envelope (following EventEnvelope pattern)
    let event_id = Uuid::new_v4();
    let envelope = EventEnvelope::with_event_id(
        event_id,
        tenant_id.to_string(),
        "ar".to_string(),
        posting_request.clone(),
    )
    .with_source_version("1.0.0".to_string());

    // Serialize envelope to JSON bytes
    let payload =
        serde_json::to_vec(&envelope).expect("Failed to serialize event envelope");

    // ✅ BOUNDARY TEST: Publish to REAL NATS subject (not calling service directly!)
    bus.publish("gl.events.posting.requested", payload)
        .await
        .expect("Failed to publish event to NATS");

    // Wait for consumer to process the event (poll processed_events table)
    let processed = wait_for_event_processing(&pool, event_id, 10).await;
    assert!(
        processed,
        "Event was not processed within 10 seconds - consumer may not be running"
    );

    // Assert: Journal entry was created
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(tenant_id)
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal entries");

    assert_eq!(entry_count, 1, "Expected exactly 1 journal entry");

    // Assert: Journal lines were created
    let line_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_lines WHERE entry_id IN (SELECT id FROM journal_entries WHERE source_event_id = $1)",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal lines");

    assert_eq!(line_count, 2, "Expected 2 journal lines");

    // Assert: Account balances were updated
    let balance_ar = balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
        .await
        .expect("Failed to query AR balance")
        .expect("AR balance should exist");

    assert_eq!(
        balance_ar.debit_total_minor, 150000,
        "AR debit balance should be $1500.00 (150000 minor units)"
    );
    assert_eq!(balance_ar.credit_total_minor, 0, "AR credit should be 0");
    assert_eq!(
        balance_ar.net_balance_minor, 150000,
        "AR net balance should be 150000 (debit positive)"
    );

    let balance_revenue = balance_repo::find_by_grain(&pool, tenant_id, period_id, "4000", "USD")
        .await
        .expect("Failed to query revenue balance")
        .expect("Revenue balance should exist");

    assert_eq!(balance_revenue.debit_total_minor, 0, "Revenue debit should be 0");
    assert_eq!(
        balance_revenue.credit_total_minor, 150000,
        "Revenue credit balance should be $1500.00"
    );
    assert_eq!(
        balance_revenue.net_balance_minor, -150000,
        "Revenue net balance should be -150000 (credit positive becomes negative in net)"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_nats_posting_replay_safety() {
    // Setup
    let pool = get_test_pool().await;
    let bus = setup_nats_bus().await;
    let tenant_id = "tenant-boundary-nats-replay";

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup COA
    insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        false,
    )
    .await;

    // Create posting request
    let posting_request = GlPostingRequestV1 {
        posting_date: "2024-02-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv-replay-001".to_string(),
        description: "Replay safety test".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 1000.00,
                credit: 0.0,
                memo: None,
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 1000.00,
                memo: None,
                dimensions: None,
            },
        ],
    };

    // Use the SAME event_id for both publishes (simulating replay)
    let event_id = Uuid::new_v4();

    // First publish
    let envelope1 = EventEnvelope::with_event_id(
        event_id,
        tenant_id.to_string(),
        "ar".to_string(),
        posting_request.clone(),
    );
    let payload1 = serde_json::to_vec(&envelope1).unwrap();
    bus.publish("gl.events.posting.requested", payload1)
        .await
        .expect("Failed to publish first event");

    // Wait for processing
    let processed1 = wait_for_event_processing(&pool, event_id, 10).await;
    assert!(processed1, "First event should be processed");

    // Check balances after first publish
    let balance_ar_first =
        balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
            .await
            .expect("Failed to query AR balance")
            .expect("AR balance should exist after first publish");

    assert_eq!(
        balance_ar_first.debit_total_minor, 100000,
        "AR balance should be 100000 after first publish"
    );

    // Second publish (REPLAY - same event_id)
    let envelope2 = EventEnvelope::with_event_id(
        event_id,
        tenant_id.to_string(),
        "ar".to_string(),
        posting_request.clone(),
    );
    let payload2 = serde_json::to_vec(&envelope2).unwrap();
    bus.publish("gl.events.posting.requested", payload2)
        .await
        .expect("Failed to publish second (replay) event");

    // Wait a bit for potential (but expected to be ignored) processing
    sleep(Duration::from_secs(2)).await;

    // Assert: Balances did NOT change (idempotency!)
    let balance_ar_after_replay =
        balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
            .await
            .expect("Failed to query AR balance")
            .expect("AR balance should still exist");

    assert_eq!(
        balance_ar_after_replay.debit_total_minor, 100000,
        "AR balance should STILL be 100000 after replay (idempotency)"
    );

    // Assert: Only one journal entry exists (not two)
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(tenant_id)
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal entries");

    assert_eq!(
        entry_count, 1,
        "Should have exactly 1 entry (replay should not create duplicate)"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
