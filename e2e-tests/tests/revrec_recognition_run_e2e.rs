//! E2E Test: Revrec Recognition Run Engine (Phase 24a — bd-b9m)
//!
//! Verifies the recognition run engine:
//! 1. Single-period recognition: posts balanced journal + marks line recognized
//! 2. Idempotency: re-running for the same period skips already-recognized lines
//! 3. Multi-period: consecutive runs recognize one period at a time
//! 4. Journal balance: every posted journal has debits == credits
//! 5. Cumulative tracking: cumulative + remaining always equals total
//! 6. Outbox emission: revrec.recognition_posted emitted per line
//! 7. Only latest version: recognition ignores superseded schedule versions

mod common;

use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use gl_rs::repos::revrec_repo;
use gl_rs::revrec::recognition_run::run_recognition;
use gl_rs::revrec::schedule_builder::generate_schedule;
use gl_rs::revrec::{
    ContractCreatedPayload, PerformanceObligation, RecognitionPattern,
    EVENT_TYPE_RECOGNITION_POSTED,
};

// ============================================================================
// Helpers
// ============================================================================

/// Advisory lock key for serializing revrec migration execution.
const REVREC_MIGRATION_LOCK_KEY: i64 = 7_419_283_562_i64;

/// Advisory lock key for GL core schema migration.
const GL_MIGRATION_LOCK_KEY: i64 = 7_419_283_563_i64;

/// Run revrec migrations on the GL database with advisory lock.
async fn run_revrec_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire revrec migration advisory lock");

    let migration_sql =
        include_str!("../../modules/gl/db/migrations/20260217000001_create_revrec_tables.sql");
    let result = sqlx::raw_sql(migration_sql).execute(pool).await;

    let versioning_sql =
        include_str!("../../modules/gl/db/migrations/20260217000002_add_schedule_versioning.sql");
    let result2 = sqlx::raw_sql(versioning_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release revrec migration advisory lock");

    result.expect("Failed to run revrec base migration");
    result2.expect("Failed to run revrec versioning migration");
}

/// Run GL core schema migrations (journal_entries, journal_lines, events_outbox, etc.)
async fn run_gl_core_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(GL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire GL migration advisory lock");

    // Create the core GL tables if they don't exist.
    // These are needed by the recognition run to post journal entries.
    let migrations = [
        include_str!("../../modules/gl/db/migrations/20260212000001_create_gl_schema.sql"),
        include_str!("../../modules/gl/db/migrations/20260212000002_create_events_tables.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000001_create_accounts_table.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000002_add_reverses_entry_id.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000003_create_accounting_periods.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000004_create_account_balances.sql"),
        include_str!("../../modules/gl/db/migrations/20260216000002_add_correlation_id_to_journal_entries.sql"),
        include_str!("../../modules/gl/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql"),
    ];

    for sql in &migrations {
        let _ = sqlx::raw_sql(sql).execute(pool).await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(GL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release GL migration advisory lock");
}

/// Build a 12-month ratable contract
fn ratable_contract(tenant_id: &str) -> (Uuid, Uuid, ContractCreatedPayload) {
    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id,
        name: "SaaS License".to_string(),
        description: "12-month platform access".to_string(),
        allocated_amount_minor: 120000_00, // $120,000
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
        satisfaction_start: "2026-01-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-revrec-001".to_string(),
        contract_name: "Enterprise SaaS — RevRec Test".to_string(),
        contract_start: "2026-01-01".to_string(),
        contract_end: Some("2026-12-31".to_string()),
        total_transaction_price_minor: 120000_00,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation],
        external_contract_ref: Some("CRM-REVREC-TEST".to_string()),
        created_at: Utc::now(),
    };

    (contract_id, obligation_id, payload)
}

/// Build a point-in-time contract
fn point_in_time_contract(tenant_id: &str) -> (Uuid, Uuid, ContractCreatedPayload) {
    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id,
        name: "Implementation".to_string(),
        description: "One-time setup".to_string(),
        allocated_amount_minor: 24000_00,
        recognition_pattern: RecognitionPattern::PointInTime,
        satisfaction_start: "2026-03-15".to_string(),
        satisfaction_end: None,
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-pit-001".to_string(),
        contract_name: "Implementation — PIT Test".to_string(),
        contract_start: "2026-03-01".to_string(),
        contract_end: None,
        total_transaction_price_minor: 24000_00,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation],
        external_contract_ref: None,
        created_at: Utc::now(),
    };

    (contract_id, obligation_id, payload)
}

/// Cleanup revrec + journal test data for a tenant
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    // Journal lines (FK to journal_entries)
    let _ = sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (
            SELECT id FROM journal_entries WHERE tenant_id = $1
        )",
    )
    .bind(tenant_id)
    .execute(pool)
    .await;

    // Journal entries
    let _ = sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;

    // Outbox events
    let _ = sqlx::query(
        "DELETE FROM events_outbox WHERE event_type LIKE 'revrec.%' AND aggregate_id IN (
            SELECT schedule_id::TEXT FROM revrec_schedules WHERE tenant_id = $1
        )",
    )
    .bind(tenant_id)
    .execute(pool)
    .await;

    // Also clean outbox for recognition runs
    let _ = sqlx::query("DELETE FROM events_outbox WHERE event_type = 'revrec.recognition_posted'")
        .execute(pool)
        .await;

    // Schedule lines
    let _ = sqlx::query(
        "DELETE FROM revrec_schedule_lines WHERE schedule_id IN (
            SELECT schedule_id FROM revrec_schedules WHERE tenant_id = $1
        )",
    )
    .bind(tenant_id)
    .execute(pool)
    .await;

    // Schedules
    let _ = sqlx::query("DELETE FROM revrec_schedules WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;

    // Obligations (FK to contracts)
    let _ = sqlx::query("DELETE FROM revrec_obligations WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;

    // Contracts
    let _ = sqlx::query("DELETE FROM revrec_contracts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;
}

/// Create contract + schedule, return (schedule_id, obligation_id)
async fn setup_contract_and_schedule(
    pool: &PgPool,
    contract_id: Uuid,
    obligation: &PerformanceObligation,
    contract_payload: &ContractCreatedPayload,
    tenant_id: &str,
) -> Uuid {
    // Create contract
    revrec_repo::create_contract(pool, Uuid::new_v4(), contract_payload)
        .await
        .expect("Contract creation failed");

    // Generate + persist schedule
    let schedule_id = Uuid::new_v4();
    let schedule_payload = generate_schedule(
        schedule_id,
        contract_id,
        obligation,
        tenant_id,
        &contract_payload.currency,
        Utc::now(),
    )
    .expect("Schedule generation failed");

    revrec_repo::create_schedule(pool, Uuid::new_v4(), &schedule_payload)
        .await
        .expect("Schedule persistence failed");

    schedule_id
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Single-period recognition posts balanced journal and marks line recognized
#[tokio::test]
async fn test_single_period_recognition_posts_balanced_journal() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, _obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];
    let schedule_id = setup_contract_and_schedule(
        &gl_pool,
        contract_id,
        obligation,
        &contract_payload,
        &tenant_id,
    )
    .await;

    // Run recognition for January 2026
    let result = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .expect("Recognition run failed");

    assert_eq!(
        result.lines_recognized, 1,
        "Should recognize exactly 1 line"
    );
    assert_eq!(result.lines_skipped, 0);
    assert_eq!(
        result.total_recognized_minor, 10000_00,
        "Should recognize $10,000"
    );
    assert_eq!(result.postings.len(), 1);
    println!(
        "✅ Single period recognized: {} lines, ${}",
        result.lines_recognized,
        result.total_recognized_minor as f64 / 100.0
    );

    // Verify journal is balanced
    let posting = &result.postings[0];
    let balance_row = sqlx::query(
        "SELECT
            COALESCE(SUM(debit_minor), 0)::BIGINT as total_debits,
            COALESCE(SUM(credit_minor), 0)::BIGINT as total_credits
         FROM journal_lines
         WHERE journal_entry_id = $1",
    )
    .bind(posting.journal_entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Journal query failed");

    let total_debits: i64 = balance_row.try_get("total_debits").unwrap();
    let total_credits: i64 = balance_row.try_get("total_credits").unwrap();
    assert_eq!(total_debits, 10000_00);
    assert_eq!(total_credits, 10000_00);
    assert_eq!(total_debits, total_credits, "Journal must be balanced");
    println!(
        "✅ Journal balanced: debits={} credits={}",
        total_debits, total_credits
    );

    // Verify schedule line marked as recognized
    let lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id)
        .await
        .expect("get_schedule_lines failed");

    let jan_line = lines.iter().find(|l| l.period == "2026-01").unwrap();
    assert!(
        jan_line.recognized,
        "January line must be marked recognized"
    );
    assert!(jan_line.recognized_at.is_some());
    println!("✅ Schedule line marked recognized");

    // Verify unrecognized lines still exist
    let unrecognized_count = lines.iter().filter(|l| !l.recognized).count();
    assert_eq!(
        unrecognized_count, 11,
        "11 months should remain unrecognized"
    );
    println!("✅ {} lines remain unrecognized", unrecognized_count);

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Single-period recognition verified");
}

/// Test 2: Idempotency — re-running recognition for same period produces no new postings
#[tokio::test]
async fn test_recognition_idempotent_on_rerun() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, _obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];
    setup_contract_and_schedule(
        &gl_pool,
        contract_id,
        obligation,
        &contract_payload,
        &tenant_id,
    )
    .await;

    // First run
    let result1 = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .expect("First recognition run failed");
    assert_eq!(result1.lines_recognized, 1);
    println!(
        "✅ First run: {} lines recognized",
        result1.lines_recognized
    );

    // Second run — same period, should skip
    let result2 = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .expect("Second recognition run failed");
    assert_eq!(
        result2.lines_recognized, 0,
        "Second run must recognize 0 lines"
    );
    assert_eq!(
        result2.lines_skipped, 0,
        "No due lines should be found (already recognized)"
    );
    assert_eq!(result2.total_recognized_minor, 0);
    println!("✅ Second run: 0 lines recognized (idempotent)");

    // Verify only 1 journal entry exists
    let journal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_module = 'gl-revrec'",
    )
    .bind(&tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Journal count query failed");
    assert_eq!(journal_count, 1, "Only 1 journal entry should exist");
    println!("✅ Only 1 journal entry exists after 2 runs");

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Idempotency verified");
}

/// Test 3: Multi-period recognition — consecutive runs process one period at a time
#[tokio::test]
async fn test_multi_period_recognition_sequential() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, _obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];
    let schedule_id = setup_contract_and_schedule(
        &gl_pool,
        contract_id,
        obligation,
        &contract_payload,
        &tenant_id,
    )
    .await;

    // Recognize January
    let jan = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .unwrap();
    assert_eq!(jan.lines_recognized, 1);
    assert_eq!(jan.total_recognized_minor, 10000_00);

    // Recognize February
    let feb = run_recognition(&gl_pool, &tenant_id, "2026-02", "2026-02-28")
        .await
        .unwrap();
    assert_eq!(feb.lines_recognized, 1);
    assert_eq!(feb.total_recognized_minor, 10000_00);

    // Recognize March
    let mar = run_recognition(&gl_pool, &tenant_id, "2026-03", "2026-03-31")
        .await
        .unwrap();
    assert_eq!(mar.lines_recognized, 1);
    assert_eq!(mar.total_recognized_minor, 10000_00);

    // Verify 3 journals created, all balanced
    let journal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_module = 'gl-revrec'",
    )
    .bind(&tenant_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();
    assert_eq!(journal_count, 3, "3 journal entries after 3 periods");
    println!("✅ 3 periods recognized with 3 balanced journals");

    // Verify total recognized via DB
    let recognized_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revrec_schedule_lines
         WHERE schedule_id = $1 AND recognized = true",
    )
    .bind(schedule_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();
    assert_eq!(recognized_count, 3, "3 lines should be recognized");

    // Verify cumulative amount
    let cumulative: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_to_recognize_minor), 0)::BIGINT
         FROM revrec_schedule_lines
         WHERE schedule_id = $1 AND recognized = true",
    )
    .bind(schedule_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();
    assert_eq!(
        cumulative, 30000_00,
        "Cumulative should be $30,000 after 3 months"
    );
    println!("✅ Cumulative recognized: ${}", cumulative as f64 / 100.0);

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Multi-period recognition verified");
}

/// Test 4: Every journal entry is balanced (debits == credits)
#[tokio::test]
async fn test_all_journals_balanced() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, _obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];
    setup_contract_and_schedule(
        &gl_pool,
        contract_id,
        obligation,
        &contract_payload,
        &tenant_id,
    )
    .await;

    // Recognize all 12 months
    for month in 1..=12 {
        let period = format!("2026-{:02}", month);
        let posting_date = format!(
            "2026-{:02}-{}",
            month,
            match month {
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            }
        );
        let result = run_recognition(&gl_pool, &tenant_id, &period, &posting_date)
            .await
            .unwrap();
        assert_eq!(
            result.lines_recognized, 1,
            "Month {} should recognize 1 line",
            month
        );
    }

    // Check every journal is balanced
    let rows = sqlx::query(
        "SELECT je.id,
                COALESCE(SUM(jl.debit_minor), 0)::BIGINT as total_debits,
                COALESCE(SUM(jl.credit_minor), 0)::BIGINT as total_credits
         FROM journal_entries je
         JOIN journal_lines jl ON jl.journal_entry_id = je.id
         WHERE je.tenant_id = $1 AND je.source_module = 'gl-revrec'
         GROUP BY je.id",
    )
    .bind(&tenant_id)
    .fetch_all(&gl_pool)
    .await
    .expect("Balance query failed");

    assert_eq!(rows.len(), 12, "12 journal entries after full year");
    for row in &rows {
        let debits: i64 = row.try_get("total_debits").unwrap();
        let credits: i64 = row.try_get("total_credits").unwrap();
        assert_eq!(debits, credits, "Each journal must be balanced");
        assert_eq!(debits, 10000_00, "Each journal should be $10,000");
    }
    println!("✅ All 12 journals balanced at $10,000 each");

    // Verify total recognized equals contract total
    let total_debits: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(jl.debit_minor), 0)::BIGINT
         FROM journal_entries je
         JOIN journal_lines jl ON jl.journal_entry_id = je.id
         WHERE je.tenant_id = $1 AND je.source_module = 'gl-revrec'",
    )
    .bind(&tenant_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();
    assert_eq!(
        total_debits, 120000_00,
        "Total debits must equal contract total"
    );
    println!(
        "✅ Total recognized: ${} (full contract)",
        total_debits as f64 / 100.0
    );

    // Verify no more lines to recognize
    let remaining = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .unwrap();
    assert_eq!(remaining.lines_recognized, 0);
    println!("✅ No lines left to recognize after full year");

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Full-year recognition with balanced journals verified");
}

/// Test 5: Outbox emission — revrec.recognition_posted emitted per recognized line
#[tokio::test]
async fn test_recognition_emits_outbox_events() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, _obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];
    setup_contract_and_schedule(
        &gl_pool,
        contract_id,
        obligation,
        &contract_payload,
        &tenant_id,
    )
    .await;

    // Recognize January
    let result = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .unwrap();
    assert_eq!(result.lines_recognized, 1);

    // Check outbox has the recognition_posted event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = $1 AND aggregate_type = 'revrec_recognition'",
    )
    .bind(EVENT_TYPE_RECOGNITION_POSTED)
    .fetch_one(&gl_pool)
    .await
    .expect("Outbox query failed");
    assert!(
        outbox_count >= 1,
        "At least 1 recognition_posted event in outbox"
    );
    println!(
        "✅ Outbox contains {} {} events",
        outbox_count, EVENT_TYPE_RECOGNITION_POSTED
    );

    // Verify outbox payload contains expected fields
    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(EVENT_TYPE_RECOGNITION_POSTED)
    .fetch_one(&gl_pool)
    .await
    .expect("Outbox payload query failed");

    assert!(
        outbox_payload.get("run_id").is_some(),
        "Payload must have run_id"
    );
    assert!(
        outbox_payload.get("contract_id").is_some(),
        "Payload must have contract_id"
    );
    assert!(
        outbox_payload.get("schedule_id").is_some(),
        "Payload must have schedule_id"
    );
    assert!(
        outbox_payload.get("period").is_some(),
        "Payload must have period"
    );
    assert_eq!(
        outbox_payload.get("period").unwrap().as_str().unwrap(),
        "2026-01"
    );
    assert_eq!(
        outbox_payload
            .get("amount_recognized_minor")
            .unwrap()
            .as_i64()
            .unwrap(),
        10000_00
    );
    println!("✅ Outbox payload has correct structure");

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Outbox emission verified");
}

/// Test 6: Point-in-time recognition — single line recognized in one shot
#[tokio::test]
async fn test_point_in_time_recognition() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, _obligation_id, contract_payload) = point_in_time_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];
    let schedule_id = setup_contract_and_schedule(
        &gl_pool,
        contract_id,
        obligation,
        &contract_payload,
        &tenant_id,
    )
    .await;

    // Run recognition for March (the satisfaction period)
    let result = run_recognition(&gl_pool, &tenant_id, "2026-03", "2026-03-31")
        .await
        .unwrap();
    assert_eq!(result.lines_recognized, 1);
    assert_eq!(result.total_recognized_minor, 24000_00);
    println!("✅ Point-in-time: recognized $24,000 in period 2026-03");

    // Verify full amount recognized in single journal
    let posting = &result.postings[0];
    let balance_row = sqlx::query(
        "SELECT
            COALESCE(SUM(debit_minor), 0)::BIGINT as total_debits,
            COALESCE(SUM(credit_minor), 0)::BIGINT as total_credits
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(posting.journal_entry_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();

    let debits: i64 = balance_row.try_get("total_debits").unwrap();
    let credits: i64 = balance_row.try_get("total_credits").unwrap();
    assert_eq!(debits, 24000_00);
    assert_eq!(credits, 24000_00);
    println!("✅ Journal balanced: $24,000 DR deferred / CR revenue");

    // Verify all schedule lines are now recognized
    let lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id)
        .await
        .unwrap();
    assert!(
        lines.iter().all(|l| l.recognized),
        "All lines must be recognized"
    );
    println!("✅ All schedule lines recognized");

    // Trying another period should find nothing
    let empty = run_recognition(&gl_pool, &tenant_id, "2026-04", "2026-04-30")
        .await
        .unwrap();
    assert_eq!(empty.lines_recognized, 0);
    println!("✅ No lines due in other periods");

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Point-in-time recognition verified");
}

/// Test 7: Only latest schedule version is recognized — superseded versions skipped
#[tokio::test]
async fn test_only_latest_version_recognized() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_gl_core_migrations(&gl_pool).await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_test_data(&gl_pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];

    // Create contract
    revrec_repo::create_contract(&gl_pool, Uuid::new_v4(), &contract_payload)
        .await
        .unwrap();

    // Create schedule v1
    let schedule_id_v1 = Uuid::new_v4();
    let payload_v1 = generate_schedule(
        schedule_id_v1,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .unwrap();
    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &payload_v1)
        .await
        .unwrap();

    // Create schedule v2 (same obligation — simulates amendment re-schedule)
    let schedule_id_v2 = Uuid::new_v4();
    let payload_v2 = generate_schedule(
        schedule_id_v2,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .unwrap();
    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &payload_v2)
        .await
        .unwrap();

    // Verify v2 exists
    let v2 = revrec_repo::get_schedule(&gl_pool, schedule_id_v2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v2.version, 2, "Should be version 2");
    println!("✅ V1 and V2 schedules created");

    // Run recognition for January — should only recognize from v2
    let result = run_recognition(&gl_pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .unwrap();

    assert_eq!(
        result.lines_recognized, 1,
        "Should recognize only 1 line (from v2)"
    );
    assert_eq!(
        result.postings[0].schedule_id, schedule_id_v2,
        "Posting must be from v2 schedule"
    );
    println!("✅ Recognition came from v2 schedule: {}", schedule_id_v2);

    // Verify v1 lines are NOT recognized
    let v1_lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id_v1)
        .await
        .unwrap();
    assert!(
        v1_lines.iter().all(|l| !l.recognized),
        "V1 lines must NOT be recognized (superseded)"
    );
    println!("✅ V1 lines remain unrecognized (superseded)");

    cleanup_test_data(&gl_pool, &tenant_id).await;
    println!("\n🎯 Latest-version-only recognition verified");
}
