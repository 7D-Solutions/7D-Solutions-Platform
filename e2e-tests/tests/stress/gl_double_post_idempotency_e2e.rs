//! Stress test: GL double-post — 50 concurrent posts prove exactly-once journal entry
//!
//! Proves that 50 concurrent attempts to post the same GL event produce exactly
//! one journal entry. The dedup mechanism is: `processed_events.event_id` UNIQUE
//! constraint + pre-check via `processed_repo::exists()`. Under concurrency, the
//! TOCTOU race between the exists-check and the insert is caught by the UNIQUE
//! constraint, producing a DB error for the losing transaction.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- gl_double_post_idempotency_e2e --nocapture
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
        .max_connections(10)
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

/// Build a balanced GL posting request for today's date.
fn build_posting_payload() -> GlPostingRequestV1 {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    GlPostingRequestV1 {
        posting_date: today,
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("stress_inv_{}", Uuid::new_v4()),
        description: "Stress test: double-post idempotency".to_string(),
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

#[derive(Debug)]
struct PostOutcome {
    success: bool,
    is_duplicate: bool,
    is_db_error: bool,
    error_msg: Option<String>,
}

#[tokio::test]
async fn gl_double_post_idempotency_e2e() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-tenant-{}", Uuid::new_v4());

    // --- Seed: accounts + period ---
    setup_accounts(&pool, &tenant_id).await;
    setup_period(&pool, &tenant_id).await;

    // Stable event_id — ALL 50 concurrent posts use the SAME event_id
    let event_id = Uuid::new_v4();
    let payload = build_posting_payload();

    println!(
        "seeded: tenant={}, event_id={}, source_doc_id={}",
        tenant_id, event_id, payload.source_doc_id
    );

    // --- Fire 50 concurrent GL posting requests with the same event_id ---
    println!("\n--- {} concurrent GL posts with same event_id ---", CONCURRENCY);

    let pool = Arc::new(pool);
    let tenant_id = Arc::new(tenant_id);
    let payload = Arc::new(payload);
    let start = Instant::now();

    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|_| {
            let pool = Arc::clone(&pool);
            let tenant_id = Arc::clone(&tenant_id);
            let payload = Arc::clone(&payload);
            tokio::spawn(async move {
                match journal_service::process_gl_posting_request(
                    &pool,
                    event_id,
                    &tenant_id,
                    "stress-test",
                    "gl.events.posting.requested",
                    &payload,
                    None,
                )
                .await
                {
                    Ok(_entry_id) => PostOutcome {
                        success: true,
                        is_duplicate: false,
                        is_db_error: false,
                        error_msg: None,
                    },
                    Err(journal_service::JournalError::DuplicateEvent(_)) => PostOutcome {
                        success: false,
                        is_duplicate: true,
                        is_db_error: false,
                        error_msg: None,
                    },
                    Err(journal_service::JournalError::Database(e)) => {
                        let msg = format!("{}", e);
                        // unique_violation on processed_events is expected under TOCTOU race
                        let is_unique_violation = msg.contains("duplicate key")
                            || msg.contains("unique constraint")
                            || msg.contains("23505");
                        PostOutcome {
                            success: false,
                            is_duplicate: is_unique_violation,
                            is_db_error: !is_unique_violation,
                            error_msg: Some(msg),
                        }
                    }
                    Err(e) => PostOutcome {
                        success: false,
                        is_duplicate: false,
                        is_db_error: true,
                        error_msg: Some(format!("{}", e)),
                    },
                }
            })
        })
        .collect();

    let mut outcomes = Vec::with_capacity(CONCURRENCY);
    for h in handles {
        outcomes.push(h.await.expect("task panicked"));
    }
    let elapsed = start.elapsed();

    // --- Analyze results ---
    let success_count = outcomes.iter().filter(|o| o.success).count();
    let duplicate_count = outcomes.iter().filter(|o| o.is_duplicate).count();
    let db_error_count = outcomes.iter().filter(|o| o.is_db_error).count();

    println!("completed in {:?}", elapsed);
    println!("  successful posts: {}", success_count);
    println!("  duplicate rejections: {}", duplicate_count);
    println!("  unexpected DB errors: {}", db_error_count);

    for (i, o) in outcomes.iter().enumerate() {
        if o.is_db_error {
            println!(
                "  request {}: UNEXPECTED ERROR: {}",
                i,
                o.error_msg.as_deref().unwrap_or("unknown")
            );
        }
    }

    // --- Assertion 1: Exactly one post succeeded ---
    assert_eq!(
        success_count, 1,
        "exactly one GL post must succeed, got {}",
        success_count
    );

    // --- Assertion 2: All others were duplicate rejections (not 500s) ---
    assert_eq!(
        duplicate_count,
        CONCURRENCY - 1,
        "all other {} posts must be duplicate rejections, got {}",
        CONCURRENCY - 1,
        duplicate_count
    );

    assert_eq!(
        db_error_count, 0,
        "no unexpected DB errors expected, got {}",
        db_error_count
    );

    // --- Assertion 3: Exactly one journal entry in DB ---
    let journal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(tenant_id.as_ref())
    .bind(event_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query journal entries");

    println!("\n  DB journal entries for event {}: {}", event_id, journal_count);

    assert_eq!(
        journal_count, 1,
        "exactly one journal entry must exist for event_id {}, got {}",
        event_id, journal_count
    );

    // --- Assertion 4: Exactly one processed_events record ---
    let processed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM processed_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query processed events");

    assert_eq!(
        processed_count, 1,
        "exactly one processed_events record must exist, got {}",
        processed_count
    );

    // --- Assertion 5: Journal lines are balanced ---
    let balance_check = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(debit_minor) - SUM(credit_minor), 0)::BIGINT
         FROM journal_lines
         WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2)",
    )
    .bind(tenant_id.as_ref())
    .bind(event_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to check journal balance");

    assert_eq!(
        balance_check, 0,
        "journal lines must be balanced (debits == credits), got imbalance of {}",
        balance_check
    );

    println!("  journal lines balanced: YES");
    println!("  exactly-once invariant: PASSED");

    cleanup_tenant(pool.as_ref(), &tenant_id).await;
}
