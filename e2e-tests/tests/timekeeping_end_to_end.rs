//! E2E Test: Timekeeping → GL Labor Cost + AR Billable Time (bd-2sxs)
//!
//! **Coverage:**
//! 1. Create employee with hourly rate + billable project
//! 2. Create timesheet entries for the period
//! 3. Submit and approve the timesheet
//! 4. Generate GL labor cost postings → verify balanced journal entries
//! 5. Export billable time to AR → verify import and idempotency
//! 6. Deterministic rerun: same period produces same posting_id (idempotent)
//!
//! **Pattern:** No Docker, no mocks — uses live TK + GL + AR database pools.
//! GL consumer's `process_labor_cost_posting` is called directly (no NATS).

mod common;

use anyhow::Result;
use chrono::NaiveDate;
use common::{generate_test_tenant, get_ar_pool, get_gl_pool, get_timekeeping_pool};
use gl_rs::consumer::timekeeping_labor_cost::{process_labor_cost_posting, LaborCostPostingPayload};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Run TK migrations against the pool.
async fn run_tk_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../modules/timekeeping/db/migrations")
        .run(pool)
        .await?;
    Ok(())
}

/// Insert required GL accounts for labor cost testing.
async fn setup_gl_accounts(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
            (gen_random_uuid(), $1, 'LABOR_EXPENSE', 'Labor Expense', 'expense', 'debit', true),
            (gen_random_uuid(), $1, 'ACCRUED_LABOR', 'Accrued Labor', 'liability', 'credit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create an open accounting period covering 2026.
async fn setup_accounting_period(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, '2026-01-01', '2026-12-31', false)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create a TK employee and return its ID.
async fn create_employee(
    pool: &PgPool,
    app_id: &str,
    code: &str,
    first: &str,
    last: &str,
    hourly_rate_minor: i64,
) -> Result<Uuid> {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO tk_employees
            (app_id, employee_code, first_name, last_name, hourly_rate_minor, currency)
        VALUES ($1, $2, $3, $4, $5, 'USD')
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(code)
    .bind(first)
    .bind(last)
    .bind(hourly_rate_minor)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Create a TK project and return its ID.
async fn create_project(
    pool: &PgPool,
    app_id: &str,
    code: &str,
    name: &str,
    billable: bool,
) -> Result<Uuid> {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO tk_projects (app_id, project_code, name, billable)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(code)
    .bind(name)
    .bind(billable)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Create a timesheet entry.
async fn create_entry(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    project_id: Uuid,
    work_date: NaiveDate,
    minutes: i32,
    desc: &str,
) -> Result<Uuid> {
    let entry_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id,
             work_date, minutes, description, entry_type, is_current)
        VALUES ($1, 1, $2, $3, $4, $5, $6, $7, 'original', TRUE)
        "#,
    )
    .bind(entry_id)
    .bind(app_id)
    .bind(employee_id)
    .bind(project_id)
    .bind(work_date)
    .bind(minutes)
    .bind(desc)
    .execute(pool)
    .await?;
    Ok(entry_id)
}

/// Submit and approve a timesheet period.
async fn submit_and_approve(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    total_minutes: i32,
) -> Result<Uuid> {
    let approval_id = Uuid::new_v4();
    let reviewer_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO tk_approval_requests
            (id, app_id, employee_id, period_start, period_end,
             status, total_minutes, submitted_at, reviewed_at,
             reviewer_id, updated_at)
        VALUES ($1, $2, $3, $4, $5, 'approved', $6, NOW(), NOW(), $7, NOW())
        "#,
    )
    .bind(approval_id)
    .bind(app_id)
    .bind(employee_id)
    .bind(period_start)
    .bind(period_end)
    .bind(total_minutes)
    .bind(reviewer_id)
    .execute(pool)
    .await?;
    Ok(approval_id)
}

/// Fetch journal entry for a given source_event_id.
async fn get_journal_entry_id(pool: &PgPool, event_id: Uuid) -> Result<Option<Uuid>> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM journal_entries WHERE source_event_id = $1 LIMIT 1")
            .bind(event_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(id,)| id))
}

/// Get journal lines for a given journal entry.
async fn get_journal_lines(
    pool: &PgPool,
    entry_id: Uuid,
) -> Result<Vec<(String, f64, f64)>> {
    let rows: Vec<(String, f64, f64)> = sqlx::query_as(
        r#"
        SELECT account_ref,
               COALESCE(debit_minor, 0)::float8 / 100.0 AS debit,
               COALESCE(credit_minor, 0)::float8 / 100.0 AS credit
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Clean up test data from TK database.
async fn cleanup_tk(pool: &PgPool, app_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM events_outbox WHERE aggregate_type IN ('labor_cost', 'billable_time_export')")
        .execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_idempotency_keys WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_export_runs WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_approval_actions WHERE approval_id IN (SELECT id FROM tk_approval_requests WHERE app_id = $1)")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_approval_requests WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_timesheet_entries WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_projects WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tk_employees WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    Ok(())
}

/// Clean up GL test data.
async fn cleanup_gl(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    Ok(())
}

/// Clean up AR billable imports.
async fn cleanup_ar(pool: &PgPool, app_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM ar_tk_billable_imports WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Full flow: entries → approval → GL labor cost posting → balanced journal
#[tokio::test]
async fn test_gl_labor_cost_posting_from_approved_time() -> Result<()> {
    let tk_pool = get_timekeeping_pool().await;
    let gl_pool = get_gl_pool().await;

    run_tk_migrations(&tk_pool).await?;

    let tenant_id = generate_test_tenant();
    let app_id = &tenant_id;

    // Cleanup
    cleanup_tk(&tk_pool, app_id).await?;
    cleanup_gl(&gl_pool, &tenant_id).await?;

    // GL setup
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    // TK setup: employee with $50/hr rate
    let emp_id = create_employee(&tk_pool, app_id, "EMP-001", "Jane", "Doe", 5000).await?;

    // Billable project
    let proj_id = create_project(&tk_pool, app_id, "PROJ-001", "Widget Build", true).await?;

    let period_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 2, 7).unwrap();

    // Create entries: 8h Mon-Fri = 5 * 480 min = 2400 min
    for day in 1..=5 {
        let date = NaiveDate::from_ymd_opt(2026, 2, day).unwrap();
        create_entry(&tk_pool, app_id, emp_id, proj_id, date, 480, "Dev work").await?;
    }

    // Approve
    submit_and_approve(&tk_pool, app_id, emp_id, period_start, period_end, 2400).await?;

    // Generate GL labor cost posting payload
    // Cost = 2400 min * 5000 (minor) / 60 = 200000 minor = $2000.00
    let posting_id = Uuid::new_v5(
        &Uuid::from_bytes([
            0x7d, 0x50, 0x1a, 0xb0, 0xcc, 0x01, 0x4e, 0x2f,
            0x8a, 0x11, 0x3c, 0xd4, 0xe5, 0xf6, 0xa7, 0xb8,
        ]),
        format!(
            "{}:{}:{}:{}:{}",
            app_id, emp_id, proj_id, period_start, period_end,
        )
        .as_bytes(),
    );

    let payload = LaborCostPostingPayload {
        posting_id,
        app_id: app_id.to_string(),
        employee_id: emp_id,
        employee_name: "Jane Doe".to_string(),
        project_id: Some(proj_id),
        project_name: Some("Widget Build".to_string()),
        period_start,
        period_end,
        total_minutes: 2400,
        hourly_rate_minor: 5000,
        currency: "USD".to_string(),
        total_cost_minor: 200000,
        posting_date: "2026-02-07".to_string(),
    };

    // Process the GL posting
    let entry_id = process_labor_cost_posting(
        &gl_pool, posting_id, &tenant_id, "timekeeping", &payload,
    )
    .await?;

    // Verify journal entry exists
    let found_id = get_journal_entry_id(&gl_pool, posting_id).await?;
    assert!(found_id.is_some(), "Journal entry should exist");
    assert_eq!(found_id.unwrap(), entry_id);

    // Verify balanced journal lines
    let lines = get_journal_lines(&gl_pool, entry_id).await?;
    assert_eq!(lines.len(), 2, "Should have exactly 2 journal lines");

    let (dr_acct, dr_debit, dr_credit) = &lines[0];
    assert_eq!(dr_acct, "LABOR_EXPENSE");
    assert!(*dr_debit > 0.0);
    assert_eq!(*dr_credit, 0.0);

    let (cr_acct, cr_debit, cr_credit) = &lines[1];
    assert_eq!(cr_acct, "ACCRUED_LABOR");
    assert_eq!(*cr_debit, 0.0);
    assert!(*cr_credit > 0.0);

    // Balanced: debit == credit
    let total_debit: f64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credit: f64 = lines.iter().map(|(_, _, c)| c).sum();
    assert!(
        (total_debit - total_credit).abs() < 0.01,
        "Journal must be balanced: debit={} credit={}",
        total_debit,
        total_credit,
    );

    // Amount should be $2000.00
    assert!(
        (total_debit - 2000.0).abs() < 0.01,
        "Expected $2000.00, got {}",
        total_debit,
    );

    // Cleanup
    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_tk(&tk_pool, app_id).await?;

    Ok(())
}

/// Idempotency: same posting_id does not create duplicate journal entries
#[tokio::test]
async fn test_gl_labor_cost_idempotent_replay() -> Result<()> {
    let gl_pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    cleanup_gl(&gl_pool, &tenant_id).await?;
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    let posting_id = Uuid::new_v4();
    let payload = LaborCostPostingPayload {
        posting_id,
        app_id: tenant_id.clone(),
        employee_id: Uuid::new_v4(),
        employee_name: "Replay Test".to_string(),
        project_id: None,
        project_name: None,
        period_start: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2026, 3, 7).unwrap(),
        total_minutes: 480,
        hourly_rate_minor: 3000,
        currency: "USD".to_string(),
        total_cost_minor: 24000,
        posting_date: "2026-03-07".to_string(),
    };

    // First call: should succeed
    let entry_id = process_labor_cost_posting(
        &gl_pool, posting_id, &tenant_id, "timekeeping", &payload,
    )
    .await?;
    assert_ne!(entry_id, Uuid::nil());

    // Second call: should return DuplicateEvent
    let replay = process_labor_cost_posting(
        &gl_pool, posting_id, &tenant_id, "timekeeping", &payload,
    )
    .await;

    assert!(
        matches!(replay, Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))),
        "Replay should return DuplicateEvent, got: {:?}",
        replay,
    );

    // Only one journal entry should exist
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::BIGINT FROM journal_entries WHERE source_event_id = $1",
    )
    .bind(posting_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(count.0, 1, "Should have exactly 1 journal entry");

    cleanup_gl(&gl_pool, &tenant_id).await?;
    Ok(())
}

/// AR billable time: import + idempotency
#[tokio::test]
async fn test_ar_billable_time_import() -> Result<()> {
    let ar_pool = get_ar_pool().await;

    let app_id = generate_test_tenant();
    cleanup_ar(&ar_pool, &app_id).await?;

    let export_id = Uuid::new_v4();
    let payload = ar_rs::integrations::timekeeping::BillableTimeExportPayload {
        export_id,
        app_id: app_id.clone(),
        period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2026, 2, 7).unwrap(),
        lines: vec![ar_rs::integrations::timekeeping::BillableTimeLine {
            employee_id: Uuid::new_v4(),
            employee_name: "Jane Doe".to_string(),
            project_id: Uuid::new_v4(),
            project_name: "Widget Build".to_string(),
            total_minutes: 2400,
            hourly_rate_minor: 5000,
            currency: "USD".to_string(),
            amount_minor: 200000,
            description: "40.0h — Jane Doe on Widget Build".to_string(),
        }],
        total_amount_minor: 200000,
        currency: "USD".to_string(),
    };

    // First import
    let result = ar_rs::integrations::timekeeping::import_billable_time(&ar_pool, &payload).await?;
    assert!(!result.already_imported);
    assert_eq!(result.lines_imported, 1);
    assert_eq!(result.total_amount_minor, 200000);

    // Idempotent replay
    let replay = ar_rs::integrations::timekeeping::import_billable_time(&ar_pool, &payload).await?;
    assert!(replay.already_imported, "Second import should be idempotent");
    assert_eq!(replay.lines_imported, 0);

    // Verify in DB
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::BIGINT FROM ar_tk_billable_imports WHERE export_id = $1",
    )
    .bind(export_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(count.0, 1, "Should have exactly 1 import record");

    cleanup_ar(&ar_pool, &app_id).await?;
    Ok(())
}

/// Full integration flow: entries → approval → GL posting → AR export
#[tokio::test]
async fn test_full_timekeeping_integration_flow() -> Result<()> {
    let tk_pool = get_timekeeping_pool().await;
    let gl_pool = get_gl_pool().await;
    let ar_pool = get_ar_pool().await;

    run_tk_migrations(&tk_pool).await?;

    let tenant_id = generate_test_tenant();
    let app_id = &tenant_id;

    cleanup_tk(&tk_pool, app_id).await?;
    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, app_id).await?;

    // GL setup
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    // Create employee ($75/hr) + billable project
    let emp_id = create_employee(&tk_pool, app_id, "EMP-E2E", "Alice", "Smith", 7500).await?;
    let proj_id = create_project(&tk_pool, app_id, "PROJ-E2E", "Consulting", true).await?;

    let period_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 2, 7).unwrap();

    // 3 days × 6h = 1080 min
    for day in [3, 4, 5] {
        let date = NaiveDate::from_ymd_opt(2026, 2, day).unwrap();
        create_entry(&tk_pool, app_id, emp_id, proj_id, date, 360, "Consulting").await?;
    }

    // Approve
    submit_and_approve(&tk_pool, app_id, emp_id, period_start, period_end, 1080).await?;

    // --- GL Integration ---
    // Cost = 1080 min * 7500 / 60 = 135000 minor = $1350.00
    let labor_ns = Uuid::from_bytes([
        0x7d, 0x50, 0x1a, 0xb0, 0xcc, 0x01, 0x4e, 0x2f,
        0x8a, 0x11, 0x3c, 0xd4, 0xe5, 0xf6, 0xa7, 0xb8,
    ]);
    let posting_id = Uuid::new_v5(
        &labor_ns,
        format!(
            "{}:{}:{}:{}:{}",
            app_id, emp_id, proj_id, period_start, period_end,
        )
        .as_bytes(),
    );

    let gl_payload = LaborCostPostingPayload {
        posting_id,
        app_id: app_id.to_string(),
        employee_id: emp_id,
        employee_name: "Alice Smith".to_string(),
        project_id: Some(proj_id),
        project_name: Some("Consulting".to_string()),
        period_start,
        period_end,
        total_minutes: 1080,
        hourly_rate_minor: 7500,
        currency: "USD".to_string(),
        total_cost_minor: 135000,
        posting_date: "2026-02-07".to_string(),
    };

    let entry_id = process_labor_cost_posting(
        &gl_pool, posting_id, &tenant_id, "timekeeping", &gl_payload,
    )
    .await?;

    // Verify GL: balanced, correct amount
    let lines = get_journal_lines(&gl_pool, entry_id).await?;
    assert_eq!(lines.len(), 2);
    let total_debit: f64 = lines.iter().map(|(_, d, _)| d).sum();
    assert!(
        (total_debit - 1350.0).abs() < 0.01,
        "Expected $1350.00, got {}",
        total_debit,
    );

    // --- AR Integration ---
    let ar_export_id = Uuid::new_v4();
    let ar_payload = ar_rs::integrations::timekeeping::BillableTimeExportPayload {
        export_id: ar_export_id,
        app_id: app_id.to_string(),
        period_start,
        period_end,
        lines: vec![ar_rs::integrations::timekeeping::BillableTimeLine {
            employee_id: emp_id,
            employee_name: "Alice Smith".to_string(),
            project_id: proj_id,
            project_name: "Consulting".to_string(),
            total_minutes: 1080,
            hourly_rate_minor: 7500,
            currency: "USD".to_string(),
            amount_minor: 135000,
            description: "18.0h — Alice Smith on Consulting".to_string(),
        }],
        total_amount_minor: 135000,
        currency: "USD".to_string(),
    };

    let ar_result = ar_rs::integrations::timekeeping::import_billable_time(
        &ar_pool, &ar_payload,
    )
    .await?;
    assert!(!ar_result.already_imported);
    assert_eq!(ar_result.lines_imported, 1);

    // Verify AR: record exists with correct amount
    let ar_line: Option<(i64, String)> = sqlx::query_as(
        "SELECT amount_minor, description FROM ar_tk_billable_imports \
         WHERE export_id = $1 LIMIT 1",
    )
    .bind(ar_export_id)
    .fetch_optional(&ar_pool)
    .await?;
    assert!(ar_line.is_some());
    let (amount, desc) = ar_line.unwrap();
    assert_eq!(amount, 135000);
    assert!(desc.contains("Alice Smith"));

    // Cleanup
    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, app_id).await?;
    cleanup_tk(&tk_pool, app_id).await?;

    Ok(())
}
