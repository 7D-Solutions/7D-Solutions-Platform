//! Audit Oracle — GL module
//!
//! Asserts that every GL mutation (process_gl_posting_request, close_period) writes
//! exactly one audit_events row inside the same transaction as the mutation.
//!
//! Real database, no mocks. Run:
//!   ./scripts/cargo-slot.sh test -p gl audit_oracle -- --nocapture

mod common;

use chrono::NaiveDate;
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::services::journal_service;
use gl_rs::services::period_close_execution::close_period;
use serial_test::serial;
use uuid::Uuid;

fn unique_tenant() -> String {
    format!("gl-audit-{}", Uuid::new_v4().simple())
}

/// Count audit_events rows for a given entity_id + action.
async fn count_audit_events(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_events WHERE entity_id = $1 AND action = $2",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("count audit_events")
}

/// Fetch mutation_class for a given entity_id + action.
async fn fetch_mutation_class(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT mutation_class::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("fetch mutation_class")
}

// ============================================================================
// 1. process_gl_posting_request → exactly 1 CREATE audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_journal_entry() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;

    // Set up accounts
    setup_test_account(&pool, &tenant, "1100", "Accounts Receivable", "asset", "debit").await;
    setup_test_account(&pool, &tenant, "4000", "Revenue", "revenue", "credit").await;

    // Set up an open accounting period covering the posting date
    setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;

    let event_id = Uuid::new_v4();
    let payload = GlPostingRequestV1 {
        posting_date: "2026-06-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("inv_{}", Uuid::new_v4()),
        description: "Audit oracle test journal entry".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
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
    };

    let entry_id = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant,
        "ar",
        "ar.invoice.finalized",
        &payload,
        None,
    )
    .await
    .expect("process_gl_posting_request");

    let entity_id = entry_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "PostJournalEntry").await;
    assert_eq!(count, 1, "Expected exactly 1 audit record for PostJournalEntry");

    let mc = fetch_mutation_class(&pool, &entity_id, "PostJournalEntry").await;
    assert_eq!(mc, "CREATE", "mutation_class should be CREATE");

    let actor_id: Option<String> = sqlx::query_scalar(
        "SELECT actor_id::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(&entity_id)
    .bind("PostJournalEntry")
    .fetch_one(&pool)
    .await
    .expect("fetch actor_id");
    assert_eq!(
        actor_id.unwrap_or_default(),
        "00000000-0000-0000-0000-000000000000",
        "actor_id should be nil UUID for system writes"
    );

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}

// ============================================================================
// 2. close_period → exactly 1 STATE_TRANSITION audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_close_period() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;

    let period_id = setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
    )
    .await;

    let result = close_period(
        &pool,
        &tenant,
        period_id,
        "audit-oracle",
        Some("audit oracle close test"),
        false,
        "USD",
    )
    .await
    .expect("close_period");

    assert!(result.success, "Period close should succeed: {:?}", result.validation_report);

    let entity_id = period_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "ClosePeriod").await;
    assert_eq!(count, 1, "Expected exactly 1 audit record for ClosePeriod");

    let mc = fetch_mutation_class(&pool, &entity_id, "ClosePeriod").await;
    assert_eq!(mc, "STATE_TRANSITION", "mutation_class should be STATE_TRANSITION");

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}
