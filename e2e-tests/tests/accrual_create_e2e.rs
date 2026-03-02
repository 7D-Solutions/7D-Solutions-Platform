//! E2E Test: Accrual Template + Instance Creation (Phase 24b — bd-3qa)
//!
//! Verifies the core accrual engine guarantees:
//! 1. Template creation stores the reversal policy and accounts correctly
//! 2. Accrual instance posts exactly one balanced journal entry (debit == credit)
//! 3. Outbox event gl.accrual_created emitted atomically with instance
//! 4. Idempotent retry returns same result without duplicate postings
//! 5. Inactive template rejects accrual creation
//! 6. Deterministic IDs: same (template_id, period) always produces same accrual_id

mod common;

use common::{generate_test_tenant, get_gl_pool};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use gl_rs::accruals::{
    create_accrual_instance, create_template, CreateAccrualRequest, CreateTemplateRequest,
};
use gl_rs::events::contracts::{ReversalPolicy, EVENT_TYPE_ACCRUAL_CREATED};

// ============================================================================
// Helpers
// ============================================================================

/// Advisory lock key for serializing accrual migration execution.
const ACCRUAL_MIGRATION_LOCK_KEY: i64 = 7_419_283_563_i64;

/// Run accrual migrations on the GL database with advisory lock.
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

/// Ensure GL core tables exist (journal entries, processed events, outbox).
async fn ensure_gl_core_tables(pool: &PgPool) {
    // These tables are created by the GL service migrations and should exist
    // in the Docker-managed DB. Verify they're present.
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

/// Create a template with standard defaults for testing.
async fn create_test_template(
    pool: &PgPool,
    tenant_id: &str,
    name: &str,
    debit: &str,
    credit: &str,
    amount_minor: i64,
) -> Uuid {
    let req = CreateTemplateRequest {
        tenant_id: tenant_id.to_string(),
        name: name.to_string(),
        description: Some(format!("Test template: {}", name)),
        debit_account: debit.to_string(),
        credit_account: credit.to_string(),
        amount_minor,
        currency: "USD".to_string(),
        reversal_policy: None, // defaults to auto_reverse_next_period: true
        cashflow_class: None,  // defaults to "operating"
    };
    let result = create_template(pool, &req)
        .await
        .expect("Failed to create template");
    assert!(result.active);
    result.template_id
}

/// Create an accounting period for a given month (required for journal posting).
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

    // Check existing
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

/// Test 1: Template creation stores correct fields.
#[tokio::test]
async fn test_template_creation() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    let tenant = generate_test_tenant();

    let req = CreateTemplateRequest {
        tenant_id: tenant.clone(),
        name: "Prepaid Insurance".to_string(),
        description: Some("Monthly insurance accrual".to_string()),
        debit_account: "PREPAID_INS".to_string(),
        credit_account: "CASH".to_string(),
        amount_minor: 120000, // $1,200.00
        currency: "USD".to_string(),
        reversal_policy: Some(ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        }),
        cashflow_class: Some("operating".to_string()),
    };

    let result = create_template(&pool, &req)
        .await
        .expect("template creation failed");
    assert_eq!(result.tenant_id, tenant);
    assert_eq!(result.name, "Prepaid Insurance");
    assert!(result.active);

    // Verify DB row
    let row = sqlx::query(
        "SELECT debit_account, credit_account, amount_minor, currency, reversal_policy, cashflow_class FROM gl_accrual_templates WHERE template_id = $1",
    )
    .bind(result.template_id)
    .fetch_one(&pool)
    .await
    .expect("template not found in DB");

    assert_eq!(row.get::<String, _>("debit_account"), "PREPAID_INS");
    assert_eq!(row.get::<String, _>("credit_account"), "CASH");
    assert_eq!(row.get::<i64, _>("amount_minor"), 120000);
    assert_eq!(row.get::<String, _>("currency"), "USD");
    assert_eq!(row.get::<String, _>("cashflow_class"), "operating");

    let rp: serde_json::Value = row.get("reversal_policy");
    assert_eq!(rp["auto_reverse_next_period"], true);

    println!("✅ test_template_creation: PASS");
}

/// Test 2: Accrual instance posts exactly one balanced journal entry.
#[tokio::test]
async fn test_accrual_posts_balanced_journal() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template(
        &pool,
        &tenant,
        "Accrued Rent",
        "RENT_EXPENSE",
        "ACCRUED_RENT",
        500000, // $5,000.00
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-01").await;

    let req = CreateAccrualRequest {
        template_id,
        tenant_id: tenant.clone(),
        period: "2026-01".to_string(),
        posting_date: "2026-01-31".to_string(),
        correlation_id: None,
    };

    let result = create_accrual_instance(&pool, &req)
        .await
        .expect("accrual creation failed");

    assert!(!result.idempotent_hit);
    assert_eq!(result.status, "posted");
    assert_eq!(result.amount_minor, 500000);
    assert_eq!(result.currency, "USD");
    assert_eq!(result.period, "2026-01");

    // Verify journal entry exists
    let je = sqlx::query("SELECT currency, description FROM journal_entries WHERE id = $1")
        .bind(result.journal_entry_id)
        .fetch_one(&pool)
        .await
        .expect("journal entry not found");
    assert_eq!(je.get::<String, _>("currency"), "USD");

    // Verify exactly 2 journal lines (debit + credit)
    let lines: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT debit_minor, credit_minor, account_ref FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(result.journal_entry_id)
    .fetch_all(&pool)
    .await
    .expect("journal lines not found");

    assert_eq!(lines.len(), 2, "Expected exactly 2 journal lines");

    // Line 1: debit RENT_EXPENSE
    assert_eq!(lines[0].0, 500000, "debit_minor");
    assert_eq!(lines[0].1, 0, "credit_minor should be 0 on debit line");
    assert_eq!(lines[0].2, "RENT_EXPENSE");

    // Line 2: credit ACCRUED_RENT
    assert_eq!(lines[1].0, 0, "debit_minor should be 0 on credit line");
    assert_eq!(lines[1].1, 500000, "credit_minor");
    assert_eq!(lines[1].2, "ACCRUED_RENT");

    // Balance: total debits == total credits
    let total_debit: i64 = lines.iter().map(|l| l.0).sum();
    let total_credit: i64 = lines.iter().map(|l| l.1).sum();
    assert_eq!(total_debit, total_credit, "Journal must be balanced");

    println!("✅ test_accrual_posts_balanced_journal: PASS");
}

/// Test 3: Outbox event gl.accrual_created emitted atomically with instance.
#[tokio::test]
async fn test_accrual_emits_outbox_event() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template(
        &pool,
        &tenant,
        "Accrued Salary",
        "SALARY_EXPENSE",
        "SALARY_PAYABLE",
        1000000, // $10,000.00
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-02").await;

    let req = CreateAccrualRequest {
        template_id,
        tenant_id: tenant.clone(),
        period: "2026-02".to_string(),
        posting_date: "2026-02-28".to_string(),
        correlation_id: None,
    };

    let result = create_accrual_instance(&pool, &req)
        .await
        .expect("accrual creation failed");

    // The outbox event_id is derived from Uuid::v5("accrual_event:{template_id}:{period}")
    // Query by the instance's outbox_event_id field
    let instance =
        sqlx::query("SELECT outbox_event_id FROM gl_accrual_instances WHERE instance_id = $1")
            .bind(result.instance_id)
            .fetch_one(&pool)
            .await
            .expect("instance not found");
    let outbox_event_id: Uuid = instance.get("outbox_event_id");

    let outbox = sqlx::query(
        "SELECT event_type, aggregate_type, aggregate_id, payload FROM events_outbox WHERE event_id = $1",
    )
    .bind(outbox_event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox event not found — atomicity violated");

    assert_eq!(
        outbox.get::<String, _>("event_type"),
        EVENT_TYPE_ACCRUAL_CREATED
    );
    assert_eq!(outbox.get::<String, _>("aggregate_type"), "accrual");

    // Verify payload contains expected fields
    let payload: serde_json::Value = outbox.get("payload");
    assert_eq!(payload["tenant_id"], tenant);
    assert_eq!(payload["amount_minor"], 1000000);
    assert_eq!(payload["debit_account"], "SALARY_EXPENSE");
    assert_eq!(payload["credit_account"], "SALARY_PAYABLE");
    assert_eq!(payload["period"], "2026-02");

    println!("✅ test_accrual_emits_outbox_event: PASS");
}

/// Test 4: Idempotent retry returns same result without duplicate postings.
#[tokio::test]
async fn test_accrual_idempotency() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template(
        &pool,
        &tenant,
        "Accrued Interest",
        "INTEREST_EXPENSE",
        "INTEREST_PAYABLE",
        250000, // $2,500.00
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-03").await;

    let req = CreateAccrualRequest {
        template_id,
        tenant_id: tenant.clone(),
        period: "2026-03".to_string(),
        posting_date: "2026-03-31".to_string(),
        correlation_id: None,
    };

    // First call
    let first = create_accrual_instance(&pool, &req)
        .await
        .expect("first accrual creation failed");
    assert!(!first.idempotent_hit);
    assert_eq!(first.status, "posted");

    // Second call — same template_id + period
    let second = create_accrual_instance(&pool, &req)
        .await
        .expect("second accrual creation failed");
    assert!(
        second.idempotent_hit,
        "Second call should be idempotent hit"
    );
    assert_eq!(second.instance_id, first.instance_id);
    assert_eq!(second.accrual_id, first.accrual_id);
    assert_eq!(second.journal_entry_id, first.journal_entry_id);
    assert_eq!(second.amount_minor, first.amount_minor);

    // Verify only 1 instance exists
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM gl_accrual_instances WHERE template_id = $1 AND period = $2",
    )
    .bind(template_id)
    .bind("2026-03")
    .fetch_one(&pool)
    .await
    .expect("count query failed");
    assert_eq!(count.0, 1, "Should have exactly 1 instance, not 2");

    // Verify only 1 journal entry
    let je_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE id = $1")
        .bind(first.journal_entry_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(je_count.0, 1);

    println!("✅ test_accrual_idempotency: PASS");
}

/// Test 5: Inactive template rejects accrual creation.
#[tokio::test]
async fn test_inactive_template_rejected() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template(
        &pool,
        &tenant,
        "Deactivation Test",
        "DR_ACCT",
        "CR_ACCT",
        100000,
    )
    .await;

    // Deactivate the template
    sqlx::query("UPDATE gl_accrual_templates SET active = FALSE WHERE template_id = $1")
        .bind(template_id)
        .execute(&pool)
        .await
        .expect("Failed to deactivate template");

    ensure_accounting_period(&pool, &tenant, "2026-04").await;

    let req = CreateAccrualRequest {
        template_id,
        tenant_id: tenant.clone(),
        period: "2026-04".to_string(),
        posting_date: "2026-04-30".to_string(),
        correlation_id: None,
    };

    let result = create_accrual_instance(&pool, &req).await;
    assert!(result.is_err(), "Inactive template should be rejected");

    let err = result.unwrap_err();
    let err_msg = format!("{}", err);
    assert!(
        err_msg.contains("inactive"),
        "Error should mention inactive: {}",
        err_msg
    );

    println!("✅ test_inactive_template_rejected: PASS");
}

/// Test 6: Deterministic IDs — same (template_id, period) produces same accrual_id.
#[tokio::test]
async fn test_deterministic_accrual_ids() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();

    let template_id = create_test_template(
        &pool,
        &tenant,
        "Determinism Test",
        "PREPAID",
        "CASH",
        300000,
    )
    .await;

    ensure_accounting_period(&pool, &tenant, "2026-05").await;

    let req = CreateAccrualRequest {
        template_id,
        tenant_id: tenant.clone(),
        period: "2026-05".to_string(),
        posting_date: "2026-05-31".to_string(),
        correlation_id: None,
    };

    let result = create_accrual_instance(&pool, &req)
        .await
        .expect("accrual creation failed");

    // Compute expected deterministic ID
    let expected_accrual_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("accrual:{}:{}", template_id, "2026-05").as_bytes(),
    );
    let expected_instance_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("instance:{}:{}", template_id, "2026-05").as_bytes(),
    );

    assert_eq!(
        result.accrual_id, expected_accrual_id,
        "accrual_id must be deterministic"
    );
    assert_eq!(
        result.instance_id, expected_instance_id,
        "instance_id must be deterministic"
    );

    println!("✅ test_deterministic_accrual_ids: PASS");
}

/// Test 7: Template validation — amount must be positive.
#[tokio::test]
async fn test_template_validation_amount_positive() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    let tenant = generate_test_tenant();

    let req = CreateTemplateRequest {
        tenant_id: tenant,
        name: "Bad Template".to_string(),
        description: None,
        debit_account: "DR".to_string(),
        credit_account: "CR".to_string(),
        amount_minor: 0, // Invalid
        currency: "USD".to_string(),
        reversal_policy: None,
        cashflow_class: None,
    };

    let result = create_template(&pool, &req).await;
    assert!(result.is_err(), "Zero amount should be rejected");

    println!("✅ test_template_validation_amount_positive: PASS");
}

/// Test 8: Template validation — debit and credit must differ.
#[tokio::test]
async fn test_template_validation_accounts_differ() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    let tenant = generate_test_tenant();

    let req = CreateTemplateRequest {
        tenant_id: tenant,
        name: "Same Account".to_string(),
        description: None,
        debit_account: "SAME_ACCT".to_string(),
        credit_account: "SAME_ACCT".to_string(),
        amount_minor: 100000,
        currency: "USD".to_string(),
        reversal_policy: None,
        cashflow_class: None,
    };

    let result = create_template(&pool, &req).await;
    assert!(
        result.is_err(),
        "Same debit/credit accounts should be rejected"
    );

    println!("✅ test_template_validation_accounts_differ: PASS");
}
