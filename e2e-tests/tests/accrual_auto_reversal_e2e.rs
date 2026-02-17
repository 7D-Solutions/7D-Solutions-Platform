//! E2E Test: Accrual Auto-Reversal at Period Open (Phase 24b — bd-2ob)
//!
//! Verifies the auto-reversal engine guarantees:
//! 1. Each accrual with auto_reverse_next_period=true produces exactly one reversal
//! 2. Reversal journal swaps debit/credit and is balanced
//! 3. Outbox event gl.accrual_reversed emitted atomically with reversal
//! 4. Replays do not duplicate reversals (idempotency)
//! 5. Reversal linkage: reversal record references original accrual
//! 6. Accruals without auto-reverse policy are not reversed
//! 7. Accruals already reversed are not double-reversed

mod common;

use common::{generate_test_tenant, get_gl_pool};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use gl_rs::accruals::{
    create_accrual_instance, create_template, execute_auto_reversals, CreateAccrualRequest,
    CreateTemplateRequest, ExecuteReversalsRequest,
};
use gl_rs::events::contracts::{
    ReversalPolicy, EVENT_TYPE_ACCRUAL_REVERSED,
};

// ============================================================================
// Helpers
// ============================================================================

/// Advisory lock key for serializing accrual migration execution.
const ACCRUAL_MIGRATION_LOCK_KEY: i64 = 7_419_283_563_i64;
const REVERSAL_MIGRATION_LOCK_KEY: i64 = 7_419_283_564_i64;

async fn run_accrual_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(ACCRUAL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire accrual migration advisory lock");

    let migration_sql =
        include_str!("../../modules/gl/db/migrations/20260217000004_create_accrual_tables.sql");
    let result = sqlx::raw_sql(migration_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ACCRUAL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release accrual migration advisory lock");

    result.expect("Failed to run accrual migration");
}

async fn run_reversal_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(REVERSAL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire reversal migration advisory lock");

    let migration_sql =
        include_str!("../../modules/gl/db/migrations/20260217000005_create_accrual_reversals.sql");
    let result = sqlx::raw_sql(migration_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REVERSAL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release reversal migration advisory lock");

    result.expect("Failed to run reversal migration");
}

async fn ensure_gl_core_tables(pool: &PgPool) {
    sqlx::query("SELECT 1 FROM journal_entries LIMIT 0")
        .execute(pool)
        .await
        .expect("journal_entries table must exist");
    sqlx::query("SELECT 1 FROM journal_lines LIMIT 0")
        .execute(pool)
        .await
        .expect("journal_lines table must exist");
    sqlx::query("SELECT 1 FROM processed_events LIMIT 0")
        .execute(pool)
        .await
        .expect("processed_events table must exist");
    sqlx::query("SELECT 1 FROM events_outbox LIMIT 0")
        .execute(pool)
        .await
        .expect("events_outbox table must exist");
}

async fn create_test_template_with_policy(
    pool: &PgPool,
    tenant_id: &str,
    name: &str,
    debit: &str,
    credit: &str,
    amount_minor: i64,
    reversal_policy: ReversalPolicy,
) -> Uuid {
    let req = CreateTemplateRequest {
        tenant_id: tenant_id.to_string(),
        name: name.to_string(),
        description: Some(format!("Test template: {}", name)),
        debit_account: debit.to_string(),
        credit_account: credit.to_string(),
        amount_minor,
        currency: "USD".to_string(),
        reversal_policy: Some(reversal_policy),
        cashflow_class: None,
    };
    let result = create_template(pool, &req)
        .await
        .expect("Failed to create template");
    result.template_id
}

async fn ensure_accounting_period(pool: &PgPool, tenant_id: &str, period: &str) -> Uuid {
    let (year, month): (i32, u32) = {
        let parts: Vec<&str> = period.split('-').collect();
        (parts[0].parse().unwrap(), parts[1].parse().unwrap())
    };

    let start = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let end = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
    }
    .pred_opt()
    .unwrap();

    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM accounting_periods WHERE tenant_id = $1 AND period_start = $2 AND period_end = $3",
    )
    .bind(tenant_id)
    .bind(start)
    .bind(end)
    .fetch_optional(pool)
    .await
    .expect("Failed to check accounting period");

    if let Some((id,)) = existing {
        return id;
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed) VALUES ($1, $2, $3, $4, false)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(start)
    .bind(end)
    .execute(pool)
    .await
    .expect("Failed to create accounting period");
    id
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Auto-reversal produces exactly one reversal journal per accrual.
#[tokio::test]
async fn test_auto_reversal_produces_one_journal() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template_with_policy(
        &pool,
        &tenant,
        "Accrued Rent",
        "RENT_EXPENSE",
        "ACCRUED_RENT",
        500000,
        ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        },
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-01").await;
    ensure_accounting_period(&pool, &tenant, "2026-02").await;

    // Create accrual in Jan
    let accrual = create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-01".to_string(),
            posting_date: "2026-01-31".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    // Execute reversals for Feb
    let result = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-02".to_string(),
            reversal_date: "2026-02-01".to_string(),
        },
    )
    .await
    .expect("reversal execution failed");

    assert_eq!(result.reversals_executed, 1, "Should execute exactly 1 reversal");
    assert_eq!(result.reversals_skipped, 0, "Should skip 0");
    assert_eq!(result.results.len(), 1);

    let rev = &result.results[0];
    assert_eq!(rev.original_accrual_id, accrual.accrual_id);
    assert_eq!(rev.amount_minor, 500000);
    assert!(!rev.idempotent_hit);

    // Verify journal entry exists and is balanced
    let lines: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT debit_minor, credit_minor, account_ref FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(rev.journal_entry_id)
    .fetch_all(&pool)
    .await
    .expect("journal lines not found");

    assert_eq!(lines.len(), 2, "Reversal should have 2 journal lines");

    // Reversal swaps accounts: DR ACCRUED_RENT (was credit), CR RENT_EXPENSE (was debit)
    assert_eq!(lines[0].2, "ACCRUED_RENT", "Debit should be original credit account");
    assert_eq!(lines[0].0, 500000, "Debit amount");
    assert_eq!(lines[0].1, 0);
    assert_eq!(lines[1].2, "RENT_EXPENSE", "Credit should be original debit account");
    assert_eq!(lines[1].0, 0);
    assert_eq!(lines[1].1, 500000, "Credit amount");

    let total_debit: i64 = lines.iter().map(|l| l.0).sum();
    let total_credit: i64 = lines.iter().map(|l| l.1).sum();
    assert_eq!(total_debit, total_credit, "Reversal journal must be balanced");

    println!("✅ test_auto_reversal_produces_one_journal: PASS");
}

/// Test 2: Outbox event gl.accrual_reversed emitted atomically.
#[tokio::test]
async fn test_reversal_emits_outbox_event() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template_with_policy(
        &pool,
        &tenant,
        "Accrued Salary",
        "SALARY_EXP",
        "SALARY_PAY",
        1000000,
        ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        },
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-03").await;
    ensure_accounting_period(&pool, &tenant, "2026-04").await;

    let accrual = create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-03".to_string(),
            posting_date: "2026-03-31".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    let result = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-04".to_string(),
            reversal_date: "2026-04-01".to_string(),
        },
    )
    .await
    .expect("reversal execution failed");

    assert_eq!(result.reversals_executed, 1);

    // Find the reversal's outbox event
    let reversal_row = sqlx::query(
        "SELECT outbox_event_id FROM gl_accrual_reversals WHERE original_accrual_id = $1",
    )
    .bind(accrual.accrual_id)
    .fetch_one(&pool)
    .await
    .expect("reversal record not found");
    let outbox_event_id: Uuid = reversal_row.get("outbox_event_id");

    let outbox = sqlx::query(
        "SELECT event_type, aggregate_type, aggregate_id, payload, mutation_class FROM events_outbox WHERE event_id = $1",
    )
    .bind(outbox_event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox event not found — atomicity violated");

    assert_eq!(
        outbox.get::<String, _>("event_type"),
        EVENT_TYPE_ACCRUAL_REVERSED,
    );
    assert_eq!(outbox.get::<String, _>("aggregate_type"), "accrual");
    assert_eq!(
        outbox.get::<Option<String>, _>("mutation_class").as_deref(),
        Some("REVERSAL"),
    );

    let payload: serde_json::Value = outbox.get("payload");
    assert_eq!(payload["tenant_id"], tenant);
    assert_eq!(payload["amount_minor"], 1000000);
    assert_eq!(payload["reason"], "auto_reverse_next_period");
    assert_eq!(payload["reversal_period"], "2026-04");
    assert_eq!(
        payload["original_accrual_id"],
        accrual.accrual_id.to_string()
    );

    println!("✅ test_reversal_emits_outbox_event: PASS");
}

/// Test 3: Replay does not duplicate reversals (idempotency).
#[tokio::test]
async fn test_reversal_replay_idempotency() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template_with_policy(
        &pool,
        &tenant,
        "Accrued Interest",
        "INT_EXP",
        "INT_PAY",
        250000,
        ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        },
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-05").await;
    ensure_accounting_period(&pool, &tenant, "2026-06").await;

    create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-05".to_string(),
            posting_date: "2026-05-31".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    let reversal_req = ExecuteReversalsRequest {
        tenant_id: tenant.clone(),
        target_period: "2026-06".to_string(),
        reversal_date: "2026-06-01".to_string(),
    };

    // First execution
    let first = execute_auto_reversals(&pool, &reversal_req)
        .await
        .expect("first reversal failed");
    assert_eq!(first.reversals_executed, 1);
    assert_eq!(first.reversals_skipped, 0);

    // Second execution (replay)
    let second = execute_auto_reversals(&pool, &reversal_req)
        .await
        .expect("second reversal failed");
    assert_eq!(second.reversals_executed, 0, "Replay should execute 0 new reversals");
    assert_eq!(second.reversals_skipped, 0, "Already reversed — not even a candidate");

    // Verify only 1 reversal record exists
    let reversal_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM gl_accrual_reversals WHERE tenant_id = $1 AND reversal_period = $2",
    )
    .bind(&tenant)
    .bind("2026-06")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reversal_count.0, 1, "Should have exactly 1 reversal record");

    // Verify only 1 reversal journal entry (not 2)
    let je_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_subject = 'accrual_reversal'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(je_count.0, 1, "Should have exactly 1 reversal journal entry");

    println!("✅ test_reversal_replay_idempotency: PASS");
}

/// Test 4: Reversal linkage — reversal record references original accrual.
#[tokio::test]
async fn test_reversal_linkage_preserved() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template_with_policy(
        &pool,
        &tenant,
        "Prepaid Insurance",
        "PREPAID_INS",
        "CASH",
        120000,
        ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        },
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-07").await;
    ensure_accounting_period(&pool, &tenant, "2026-08").await;

    let accrual = create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-07".to_string(),
            posting_date: "2026-07-31".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-08".to_string(),
            reversal_date: "2026-08-01".to_string(),
        },
    )
    .await
    .expect("reversal execution failed");

    // Verify reversal record linkage
    let rev = sqlx::query(
        r#"
        SELECT reversal_id, original_accrual_id, original_instance_id,
               reversal_period, debit_account, credit_account,
               amount_minor, currency, reason
        FROM gl_accrual_reversals
        WHERE original_accrual_id = $1
        "#,
    )
    .bind(accrual.accrual_id)
    .fetch_one(&pool)
    .await
    .expect("reversal record not found");

    assert_eq!(rev.get::<Uuid, _>("original_accrual_id"), accrual.accrual_id);
    assert_eq!(rev.get::<Uuid, _>("original_instance_id"), accrual.instance_id);
    assert_eq!(rev.get::<String, _>("reversal_period"), "2026-08");
    // Accounts swapped
    assert_eq!(rev.get::<String, _>("debit_account"), "CASH");
    assert_eq!(rev.get::<String, _>("credit_account"), "PREPAID_INS");
    assert_eq!(rev.get::<i64, _>("amount_minor"), 120000);
    assert_eq!(rev.get::<String, _>("currency"), "USD");
    assert_eq!(rev.get::<String, _>("reason"), "auto_reverse_next_period");

    // Verify deterministic reversal_id
    let expected_reversal_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("reversal:{}", accrual.accrual_id).as_bytes(),
    );
    assert_eq!(
        rev.get::<Uuid, _>("reversal_id"),
        expected_reversal_id,
        "reversal_id must be deterministic"
    );

    // Verify original accrual status updated to 'reversed'
    let instance_status: (String,) = sqlx::query_as(
        "SELECT status FROM gl_accrual_instances WHERE instance_id = $1",
    )
    .bind(accrual.instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(instance_status.0, "reversed", "Original accrual should be marked reversed");

    println!("✅ test_reversal_linkage_preserved: PASS");
}

/// Test 5: Accruals without auto-reverse policy are not reversed.
#[tokio::test]
async fn test_no_reverse_when_policy_disabled() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    // Create template with auto_reverse = false
    let template_id = create_test_template_with_policy(
        &pool,
        &tenant,
        "No-Reverse Accrual",
        "PREPAID",
        "CASH",
        300000,
        ReversalPolicy {
            auto_reverse_next_period: false,
            reverse_on_date: Some("2026-12-31".to_string()),
        },
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-09").await;
    ensure_accounting_period(&pool, &tenant, "2026-10").await;

    create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-09".to_string(),
            posting_date: "2026-09-30".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    // Execute reversals for Oct — should find nothing
    let result = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-10".to_string(),
            reversal_date: "2026-10-01".to_string(),
        },
    )
    .await
    .expect("reversal execution failed");

    assert_eq!(result.reversals_executed, 0, "No auto-reverse accruals should be found");
    assert_eq!(result.reversals_skipped, 0);
    assert!(result.results.is_empty());

    println!("✅ test_no_reverse_when_policy_disabled: PASS");
}

/// Test 6: Multiple accruals in same period all reversed.
#[tokio::test]
async fn test_multiple_accruals_all_reversed() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let policy = ReversalPolicy {
        auto_reverse_next_period: true,
        reverse_on_date: None,
    };

    let t1 = create_test_template_with_policy(
        &pool, &tenant, "Accrual A", "EXP_A", "LIAB_A", 100000, policy.clone(),
    )
    .await;
    let t2 = create_test_template_with_policy(
        &pool, &tenant, "Accrual B", "EXP_B", "LIAB_B", 200000, policy.clone(),
    )
    .await;
    let t3 = create_test_template_with_policy(
        &pool, &tenant, "Accrual C", "EXP_C", "LIAB_C", 300000, policy,
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-01").await;
    ensure_accounting_period(&pool, &tenant, "2026-02").await;

    for (tid, period) in [(t1, "2026-01"), (t2, "2026-01"), (t3, "2026-01")] {
        create_accrual_instance(
            &pool,
            &CreateAccrualRequest {
                template_id: tid,
                tenant_id: tenant.clone(),
                period: period.to_string(),
                posting_date: "2026-01-31".to_string(),
                correlation_id: None,
            },
        )
        .await
        .expect("accrual creation failed");
    }

    let result = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-02".to_string(),
            reversal_date: "2026-02-01".to_string(),
        },
    )
    .await
    .expect("reversal execution failed");

    assert_eq!(result.reversals_executed, 3, "All 3 accruals should be reversed");
    assert_eq!(result.results.len(), 3);

    // Verify total amounts
    let amounts: Vec<i64> = result.results.iter().map(|r| r.amount_minor).collect();
    assert!(amounts.contains(&100000));
    assert!(amounts.contains(&200000));
    assert!(amounts.contains(&300000));

    println!("✅ test_multiple_accruals_all_reversed: PASS");
}

/// Test 7: processed_events dedupe prevents duplicate reversal on concurrent replay.
#[tokio::test]
async fn test_processed_events_dedupe() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template_with_policy(
        &pool,
        &tenant,
        "Dedupe Test",
        "DR_DEDUPE",
        "CR_DEDUPE",
        400000,
        ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        },
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-11").await;
    ensure_accounting_period(&pool, &tenant, "2026-12").await;

    let accrual = create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-11".to_string(),
            posting_date: "2026-11-30".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    // Execute reversal
    let first = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-12".to_string(),
            reversal_date: "2026-12-01".to_string(),
        },
    )
    .await
    .expect("first reversal failed");
    assert_eq!(first.reversals_executed, 1);

    // Verify processed_events entry exists for the reversal event_id
    let expected_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("reversal_event:{}", accrual.accrual_id).as_bytes(),
    );
    let processed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM processed_events WHERE event_id = $1)",
    )
    .bind(expected_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(processed, "processed_events should contain the reversal event_id");

    println!("✅ test_processed_events_dedupe: PASS");
}
