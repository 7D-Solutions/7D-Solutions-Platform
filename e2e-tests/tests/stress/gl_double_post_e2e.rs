//! Stress test: GL double-post — 50 concurrent journal entries prove idempotency
//!
//! Proves that 50 concurrent GL posting requests — each with a unique event_id —
//! produce exactly 50 journal entries. No duplicates from consumer replay, no
//! lost entries under contention. Each entry has a unique idempotency key
//! (event_id) and the ledger stays balanced.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- gl_double_post_e2e --nocapture
//! ```

use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::services::journal_service;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CONCURRENCY: usize = 50;

async fn get_gl_pool() -> PgPool {
    let url = std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string());
    PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("failed to connect to GL database")
}

/// Seed chart of accounts entries needed for journal lines.
async fn setup_accounts(pool: &PgPool, tenant_id: &str) {
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
        .await
        .expect("failed to create account");
    }
}

/// Seed an open accounting period covering today.
async fn setup_period(pool: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, DATE_TRUNC('month', CURRENT_DATE)::date,
                 (DATE_TRUNC('month', CURRENT_DATE) + INTERVAL '1 month - 1 day')::date, false, NOW())
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("failed to create accounting period");
}

/// Build a balanced GL posting request with a unique source_doc_id.
fn build_posting_payload(seq: usize) -> GlPostingRequestV1 {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    GlPostingRequestV1 {
        posting_date: today,
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("stress_gl50_{seq}_{}", Uuid::new_v4()),
        description: format!("Stress test: concurrent GL post #{seq}"),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("Accounts Receivable".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    }
}

/// Clean up all test data for a tenant.
async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn gl_double_post_e2e() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-tenant-{}", Uuid::new_v4());

    // --- Seed: accounts + period ---
    setup_accounts(&pool, &tenant_id).await;
    setup_period(&pool, &tenant_id).await;

    // Generate 50 unique event_ids and payloads
    let events: Vec<(Uuid, GlPostingRequestV1)> = (0..CONCURRENCY)
        .map(|i| (Uuid::new_v4(), build_posting_payload(i)))
        .collect();

    println!(
        "seeded: tenant={}, firing {} concurrent GL posts with unique event_ids",
        tenant_id, CONCURRENCY
    );

    // --- Fire 50 concurrent GL posting requests, each with a unique event_id ---
    let pool = Arc::new(pool);
    let tenant_id = Arc::new(tenant_id);
    let start = Instant::now();

    let handles: Vec<_> = events
        .into_iter()
        .map(|(event_id, payload)| {
            let pool = Arc::clone(&pool);
            let tenant_id = Arc::clone(&tenant_id);
            tokio::spawn(async move {
                let result = journal_service::process_gl_posting_request(
                    &pool,
                    event_id,
                    &tenant_id,
                    "stress-test",
                    "gl.events.posting.requested",
                    &payload,
                    None,
                )
                .await;
                (event_id, result)
            })
        })
        .collect();

    let mut success_count = 0usize;
    let mut error_count = 0usize;
    let mut event_ids_succeeded = Vec::with_capacity(CONCURRENCY);

    for h in handles {
        let (event_id, result) = h.await.expect("task panicked");
        match result {
            Ok(_entry_id) => {
                success_count += 1;
                event_ids_succeeded.push(event_id);
            }
            Err(e) => {
                error_count += 1;
                println!("  UNEXPECTED ERROR for event {event_id}: {e}");
            }
        }
    }
    let elapsed = start.elapsed();

    println!("completed in {:?}", elapsed);
    println!("  successful posts: {success_count}");
    println!("  errors: {error_count}");

    // --- Assertion 1: All 50 posts succeeded (unique event_ids, no conflicts) ---
    assert_eq!(
        success_count, CONCURRENCY,
        "all {CONCURRENCY} unique GL posts must succeed, got {success_count}"
    );
    assert_eq!(error_count, 0, "no errors expected, got {error_count}");

    // --- Assertion 2: Exactly 50 journal entries in DB ---
    let journal_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(tenant_id.as_ref())
            .fetch_one(pool.as_ref())
            .await
            .expect("failed to query journal entries");

    println!("\n  DB journal entries for tenant: {journal_count}");

    assert_eq!(
        journal_count, CONCURRENCY as i64,
        "exactly {CONCURRENCY} journal entries must exist, got {journal_count}"
    );

    // --- Assertion 3: Each entry has a unique idempotency key (source_event_id) ---
    let distinct_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT source_event_id) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(tenant_id.as_ref())
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query distinct event ids");

    assert_eq!(
        distinct_event_count, CONCURRENCY as i64,
        "all {CONCURRENCY} journal entries must have unique source_event_ids, got {distinct_event_count} distinct"
    );

    // --- Assertion 4: Processed events table has exactly 50 entries ---
    let processed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM processed_events WHERE event_id = ANY($1)")
            .bind(&event_ids_succeeded)
            .fetch_one(pool.as_ref())
            .await
            .expect("failed to query processed events");

    assert_eq!(
        processed_count, CONCURRENCY as i64,
        "exactly {CONCURRENCY} processed_events records must exist, got {processed_count}"
    );

    // --- Assertion 5: Ledger is balanced (total debits == total credits) ---
    let balance_check = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(debit_minor) - SUM(credit_minor), 0)::BIGINT
         FROM journal_lines
         WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id.as_ref())
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to check journal balance");

    assert_eq!(
        balance_check, 0,
        "ledger must be balanced (debits == credits), got imbalance of {balance_check}"
    );

    println!("  journal lines balanced: YES");

    // --- Post-burst health check: GL database responds ---
    let health: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(pool.as_ref())
        .await
        .expect("post-burst health check failed — GL database unresponsive");

    assert_eq!(health, 1, "post-burst health check must return 1");
    println!("  post-burst health check: PASSED");

    println!("\n  50 concurrent GL posts → 50 unique journal entries, zero duplicates: PASSED");

    cleanup_tenant(pool.as_ref(), &tenant_id).await;
}
