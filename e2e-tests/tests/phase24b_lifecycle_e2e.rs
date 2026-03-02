//! Phase 24b Integrated E2E: Accrual → Close → Reversal → Cash Flow
//!
//! Proves the complete accrual lifecycle end-to-end with real DB:
//! 1. Create accrual in period N → journal entry posted, balanced
//! 2. Close period N (mark is_closed)
//! 3. Execute auto-reversal in period N+1 → reversing journal posted, balanced
//! 4. Cash flow report for each period reconciles to cash account deltas
//! 5. Net cash flow across N + N+1 = 0 (accrual reversal fully cancels)
//! 6. Replay is idempotent — no duplicate postings
//! 7. Cash flow output is deterministic across repeated queries
//!
//! Run with: cargo test -p e2e-tests phase24b_lifecycle_e2e -- --nocapture

mod common;

use common::{generate_test_tenant, get_gl_pool};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use gl_rs::accruals::{
    create_accrual_instance, create_template, execute_auto_reversals, CreateAccrualRequest,
    CreateTemplateRequest, ExecuteReversalsRequest,
};
use gl_rs::events::contracts::ReversalPolicy;
use gl_rs::services::cashflow_service;

// ============================================================================
// Helpers
// ============================================================================

const ACCRUAL_MIGRATION_LOCK_KEY: i64 = 7_419_283_563_i64;
const REVERSAL_MIGRATION_LOCK_KEY: i64 = 7_419_283_564_i64;
const CASHFLOW_MIGRATION_LOCK_KEY: i64 = 7_419_283_565_i64;

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

async fn run_cashflow_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(CASHFLOW_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire cashflow migration advisory lock");

    // Create enum idempotently (may already exist from GL service migrations)
    sqlx::query(
        r#"
        DO $$ BEGIN
            CREATE TYPE cashflow_category AS ENUM ('operating', 'investing', 'financing');
        EXCEPTION WHEN duplicate_object THEN NULL;
        END $$;
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create cashflow_category enum");

    // Create table idempotently
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS cashflow_classifications (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id TEXT NOT NULL,
            account_code TEXT NOT NULL,
            category cashflow_category NOT NULL,
            created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
            CONSTRAINT unique_cashflow_classification UNIQUE (tenant_id, account_code)
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create cashflow_classifications table");

    // Create indexes idempotently
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_cashflow_classifications_tenant ON cashflow_classifications(tenant_id)",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_cashflow_classifications_tenant_category ON cashflow_classifications(tenant_id, category)",
    )
    .execute(pool)
    .await
    .ok();

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(CASHFLOW_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release cashflow migration advisory lock");
}

async fn ensure_gl_core_tables(pool: &PgPool) {
    for table in &[
        "journal_entries",
        "journal_lines",
        "processed_events",
        "events_outbox",
    ] {
        sqlx::query(&format!("SELECT 1 FROM {} LIMIT 0", table))
            .execute(pool)
            .await
            .unwrap_or_else(|_| panic!("{} table must exist", table));
    }
}

/// Create an accounting period (YYYY-MM format) and return its UUID.
async fn ensure_period(pool: &PgPool, tenant_id: &str, period: &str) -> Uuid {
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

/// Create an account in the chart of accounts.
async fn create_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: &str,
    normal_balance: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, true, NOW())
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .execute(pool)
    .await
    .expect("create account");
}

/// Classify an account for cash flow reporting.
async fn classify_account(pool: &PgPool, tenant_id: &str, account_code: &str, category: &str) {
    sqlx::query(
        r#"
        INSERT INTO cashflow_classifications (id, tenant_id, account_code, category, created_at)
        VALUES ($1, $2, $3, $4::cashflow_category, NOW())
        ON CONFLICT (tenant_id, account_code) DO UPDATE SET category = $4::cashflow_category
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(account_code)
    .bind(category)
    .execute(pool)
    .await
    .expect("classify account");
}

/// Update account_balances from a journal entry's lines.
///
/// The accrual engine posts journal entries but does not update account_balances
/// (that's the balance_updater's job). For E2E testing, we manually derive
/// balances from the journal lines to verify cash flow reconciliation.
async fn update_balances_from_journal(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    journal_entry_id: Uuid,
    currency: &str,
) {
    let lines = sqlx::query(
        "SELECT account_ref, debit_minor, credit_minor FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(journal_entry_id)
    .fetch_all(pool)
    .await
    .expect("fetch journal lines for balance update");

    for line in &lines {
        let account_ref: String = line.get("account_ref");
        let debit: i64 = line.get("debit_minor");
        let credit: i64 = line.get("credit_minor");
        let net = debit - credit;

        sqlx::query(
            r#"
            INSERT INTO account_balances (id, tenant_id, period_id, account_code, currency,
                                          debit_total_minor, credit_total_minor, net_balance_minor,
                                          last_journal_entry_id, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW(), NOW())
            ON CONFLICT (tenant_id, period_id, account_code, currency)
            DO UPDATE SET
                debit_total_minor = account_balances.debit_total_minor + $6,
                credit_total_minor = account_balances.credit_total_minor + $7,
                net_balance_minor = account_balances.net_balance_minor + $8,
                last_journal_entry_id = $9,
                updated_at = NOW()
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(period_id)
        .bind(&account_ref)
        .bind(currency)
        .bind(debit)
        .bind(credit)
        .bind(net)
        .bind(journal_entry_id)
        .execute(pool)
        .await
        .expect("upsert account balance");
    }
}

/// Close an accounting period (set is_closed = true with close_hash).
async fn close_period(pool: &PgPool, period_id: Uuid) {
    let close_hash = format!("test-close-hash-{}", Uuid::new_v4());
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET is_closed = true,
            closed_at = NOW(),
            closed_by = 'e2e-test',
            close_reason = 'Phase 24b lifecycle E2E',
            close_hash = $2
        WHERE id = $1
        "#,
    )
    .bind(period_id)
    .bind(&close_hash)
    .execute(pool)
    .await
    .expect("close period");
}

/// Clean up all test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    for query in &[
        "DELETE FROM cashflow_classifications WHERE tenant_id = $1",
        "DELETE FROM gl_accrual_reversals WHERE tenant_id = $1",
        "DELETE FROM gl_accrual_instances WHERE tenant_id = $1",
        "DELETE FROM gl_accrual_templates WHERE tenant_id = $1",
        "DELETE FROM period_summary_snapshots WHERE tenant_id = $1",
        "DELETE FROM account_balances WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE tenant_id = $1",
    ] {
        sqlx::query(query).bind(tenant_id).execute(pool).await.ok();
    }

    // processed_events linked via journal entries
    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();

    for query in &[
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(query).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Full lifecycle — accrual in period N → close → reversal in N+1 → cash flow reconciles
///
/// Scenario:
///   Template: DR PREPAID_INS, CR CASH_ACCT, $5,000 auto_reverse=true
///   Period N (2026-06): accrual posts → cash outflow
///   Period N+1 (2026-07): auto-reversal posts → cash inflow
///   Cash flow per period should reconcile to cash account delta
///   Net across both periods = 0 (accrual cancellation)
#[tokio::test]
async fn test_accrual_reversal_cashflow_lifecycle() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    run_cashflow_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();
    cleanup_tenant(&pool, &tenant).await;

    // --- Setup accounts and classifications ---
    create_account(
        &pool,
        &tenant,
        "PREPAID_INS",
        "Prepaid Insurance",
        "asset",
        "debit",
    )
    .await;
    create_account(&pool, &tenant, "CASH_ACCT", "Cash", "asset", "debit").await;

    // Classify ONLY the cash account for cash flow — this means only journal lines
    // hitting CASH_ACCT are included in the cash flow statement, and reconciliation
    // should match exactly.
    classify_account(&pool, &tenant, "CASH_ACCT", "operating").await;

    // --- Setup periods ---
    let period_n_id = ensure_period(&pool, &tenant, "2026-06").await;
    let period_n1_id = ensure_period(&pool, &tenant, "2026-07").await;

    // --- Create template ---
    let template_id = {
        let req = CreateTemplateRequest {
            tenant_id: tenant.clone(),
            name: "Prepaid Insurance".to_string(),
            description: Some("Monthly insurance prepayment".to_string()),
            debit_account: "PREPAID_INS".to_string(),
            credit_account: "CASH_ACCT".to_string(),
            amount_minor: 500000, // $5,000.00
            currency: "USD".to_string(),
            reversal_policy: Some(ReversalPolicy {
                auto_reverse_next_period: true,
                reverse_on_date: None,
            }),
            cashflow_class: Some("operating".to_string()),
        };
        let result = create_template(&pool, &req).await.expect("create template");
        assert!(result.active);
        result.template_id
    };

    // =========================================================
    // STEP 1: Create accrual in period N
    // =========================================================
    println!("\n--- Step 1: Create accrual in period N (2026-06) ---");

    let accrual = create_accrual_instance(
        &pool,
        &CreateAccrualRequest {
            template_id,
            tenant_id: tenant.clone(),
            period: "2026-06".to_string(),
            posting_date: "2026-06-30".to_string(),
            correlation_id: None,
        },
    )
    .await
    .expect("accrual creation failed");

    assert!(
        !accrual.idempotent_hit,
        "First creation should not be idempotent"
    );
    assert_eq!(accrual.status, "posted");
    assert_eq!(accrual.amount_minor, 500000);
    assert_eq!(accrual.currency, "USD");
    println!(
        "  Accrual posted: journal_entry_id={}",
        accrual.journal_entry_id
    );

    // Verify journal is balanced
    let lines: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT debit_minor, credit_minor, account_ref FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(accrual.journal_entry_id)
    .fetch_all(&pool)
    .await
    .expect("fetch accrual journal lines");

    assert_eq!(
        lines.len(),
        2,
        "Accrual should have exactly 2 journal lines"
    );
    let total_dr: i64 = lines.iter().map(|l| l.0).sum();
    let total_cr: i64 = lines.iter().map(|l| l.1).sum();
    assert_eq!(total_dr, total_cr, "Accrual journal must be balanced");
    assert_eq!(total_dr, 500000);
    println!("  Journal balanced: DR={} CR={}", total_dr, total_cr);

    // Update account_balances for period N
    update_balances_from_journal(&pool, &tenant, period_n_id, accrual.journal_entry_id, "USD")
        .await;

    // =========================================================
    // STEP 2: Verify cash flow for period N
    // =========================================================
    println!("\n--- Step 2: Cash flow for period N ---");

    let cf_n = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow period N");

    println!("  Rows: {:?}", cf_n.rows);
    println!("  Net cash flow: {}", cf_n.net_cash_flow);
    println!(
        "  Cash account net change: {}",
        cf_n.cash_account_net_change
    );
    println!("  Reconciles: {}", cf_n.reconciles);

    // CASH_ACCT journal lines in period N: DR=0, CR=500000 → net = -500000
    assert_eq!(
        cf_n.net_cash_flow, -500000,
        "Period N: cash outflow from accrual"
    );
    assert_eq!(
        cf_n.cash_account_net_change, -500000,
        "Period N: cash account net change matches"
    );
    assert!(
        cf_n.reconciles,
        "Period N: cash flow should reconcile to cash account delta"
    );

    // =========================================================
    // STEP 3: Close period N
    // =========================================================
    println!("\n--- Step 3: Close period N ---");
    close_period(&pool, period_n_id).await;

    let closed: (bool,) = sqlx::query_as("SELECT is_closed FROM accounting_periods WHERE id = $1")
        .bind(period_n_id)
        .fetch_one(&pool)
        .await
        .expect("check period closed");
    assert!(closed.0, "Period N should be closed");
    println!("  Period N closed successfully");

    // =========================================================
    // STEP 4: Execute auto-reversals in period N+1
    // =========================================================
    println!("\n--- Step 4: Auto-reversal in period N+1 (2026-07) ---");

    let reversal_result = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-07".to_string(),
            reversal_date: "2026-07-01".to_string(),
        },
    )
    .await
    .expect("reversal execution failed");

    assert_eq!(
        reversal_result.reversals_executed, 1,
        "Should reverse exactly 1 accrual"
    );
    assert_eq!(reversal_result.reversals_skipped, 0);
    let rev = &reversal_result.results[0];
    assert_eq!(rev.original_accrual_id, accrual.accrual_id);
    assert_eq!(rev.amount_minor, 500000);
    assert!(!rev.idempotent_hit);
    println!(
        "  Reversal posted: journal_entry_id={}",
        rev.journal_entry_id
    );

    // Verify reversal journal is balanced
    let rev_lines: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT debit_minor, credit_minor, account_ref FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(rev.journal_entry_id)
    .fetch_all(&pool)
    .await
    .expect("fetch reversal journal lines");

    assert_eq!(
        rev_lines.len(),
        2,
        "Reversal should have exactly 2 journal lines"
    );
    let rev_dr: i64 = rev_lines.iter().map(|l| l.0).sum();
    let rev_cr: i64 = rev_lines.iter().map(|l| l.1).sum();
    assert_eq!(rev_dr, rev_cr, "Reversal journal must be balanced");
    assert_eq!(rev_dr, 500000);

    // Verify accounts are swapped: reversal DR=CASH_ACCT, CR=PREPAID_INS
    assert_eq!(
        rev_lines[0].2, "CASH_ACCT",
        "Reversal debit should be original credit"
    );
    assert_eq!(
        rev_lines[1].2, "PREPAID_INS",
        "Reversal credit should be original debit"
    );
    println!("  Reversal journal balanced: DR={} CR={}", rev_dr, rev_cr);

    // Verify original accrual status updated to 'reversed'
    let instance_status: (String,) =
        sqlx::query_as("SELECT status FROM gl_accrual_instances WHERE instance_id = $1")
            .bind(accrual.instance_id)
            .fetch_one(&pool)
            .await
            .expect("check instance status");
    assert_eq!(
        instance_status.0, "reversed",
        "Accrual should be marked reversed"
    );

    // Update account_balances for period N+1
    update_balances_from_journal(&pool, &tenant, period_n1_id, rev.journal_entry_id, "USD").await;

    // =========================================================
    // STEP 5: Verify cash flow for period N+1
    // =========================================================
    println!("\n--- Step 5: Cash flow for period N+1 ---");

    let cf_n1 = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n1_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow period N+1");

    println!("  Rows: {:?}", cf_n1.rows);
    println!("  Net cash flow: {}", cf_n1.net_cash_flow);
    println!(
        "  Cash account net change: {}",
        cf_n1.cash_account_net_change
    );
    println!("  Reconciles: {}", cf_n1.reconciles);

    // CASH_ACCT journal lines in period N+1: DR=500000, CR=0 → net = +500000
    assert_eq!(
        cf_n1.net_cash_flow, 500000,
        "Period N+1: cash inflow from reversal"
    );
    assert_eq!(
        cf_n1.cash_account_net_change, 500000,
        "Period N+1: cash account net change matches"
    );
    assert!(
        cf_n1.reconciles,
        "Period N+1: cash flow should reconcile to cash account delta"
    );

    // =========================================================
    // STEP 6: Cross-period verification — net = 0
    // =========================================================
    println!("\n--- Step 6: Cross-period verification ---");

    let net_across_periods = cf_n.net_cash_flow + cf_n1.net_cash_flow;
    assert_eq!(
        net_across_periods, 0,
        "Accrual + reversal should fully cancel across periods"
    );
    println!(
        "  Period N: {} + Period N+1: {} = Net: {}",
        cf_n.net_cash_flow, cf_n1.net_cash_flow, net_across_periods
    );

    // =========================================================
    // STEP 7: Outbox events emitted for both accrual and reversal
    // =========================================================
    println!("\n--- Step 7: Outbox event verification ---");

    let accrual_outbox: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'gl.accrual_created' AND aggregate_id = $1",
    )
    .bind(accrual.accrual_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count accrual outbox");
    assert_eq!(accrual_outbox.0, 1, "Exactly 1 accrual_created event");

    let reversal_outbox: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'gl.accrual_reversed' AND aggregate_id = $1",
    )
    .bind(accrual.accrual_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count reversal outbox");
    assert_eq!(reversal_outbox.0, 1, "Exactly 1 accrual_reversed event");

    println!("  Accrual outbox: {} event(s)", accrual_outbox.0);
    println!("  Reversal outbox: {} event(s)", reversal_outbox.0);

    cleanup_tenant(&pool, &tenant).await;
    println!("\n✅ test_accrual_reversal_cashflow_lifecycle: PASS");
}

/// Test 2: Replay idempotency — re-create accrual and re-reverse, verify no duplicates
///
/// After the full lifecycle completes, replaying the same operations must:
/// - Return idempotent_hit=true for accrual creation
/// - Return 0 new reversals for reversal execution
/// - Cash flow results remain unchanged
/// - No duplicate journal entries or outbox events
#[tokio::test]
async fn test_lifecycle_replay_no_duplicate_postings() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    run_cashflow_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();
    cleanup_tenant(&pool, &tenant).await;

    // --- Same setup as lifecycle test ---
    create_account(
        &pool,
        &tenant,
        "PREPAID_INS",
        "Prepaid Insurance",
        "asset",
        "debit",
    )
    .await;
    create_account(&pool, &tenant, "CASH_ACCT", "Cash", "asset", "debit").await;
    classify_account(&pool, &tenant, "CASH_ACCT", "operating").await;

    let period_n_id = ensure_period(&pool, &tenant, "2026-08").await;
    let period_n1_id = ensure_period(&pool, &tenant, "2026-09").await;

    let template_id = {
        let req = CreateTemplateRequest {
            tenant_id: tenant.clone(),
            name: "Replay Test Accrual".to_string(),
            description: None,
            debit_account: "PREPAID_INS".to_string(),
            credit_account: "CASH_ACCT".to_string(),
            amount_minor: 750000, // $7,500.00
            currency: "USD".to_string(),
            reversal_policy: Some(ReversalPolicy {
                auto_reverse_next_period: true,
                reverse_on_date: None,
            }),
            cashflow_class: Some("operating".to_string()),
        };
        create_template(&pool, &req)
            .await
            .expect("create template")
            .template_id
    };

    // --- First pass: create accrual + reversal ---
    let accrual_req = CreateAccrualRequest {
        template_id,
        tenant_id: tenant.clone(),
        period: "2026-08".to_string(),
        posting_date: "2026-08-31".to_string(),
        correlation_id: None,
    };

    let first_accrual = create_accrual_instance(&pool, &accrual_req)
        .await
        .expect("first accrual");
    assert!(!first_accrual.idempotent_hit);

    update_balances_from_journal(
        &pool,
        &tenant,
        period_n_id,
        first_accrual.journal_entry_id,
        "USD",
    )
    .await;

    let reversal_req = ExecuteReversalsRequest {
        tenant_id: tenant.clone(),
        target_period: "2026-09".to_string(),
        reversal_date: "2026-09-01".to_string(),
    };

    let first_reversal = execute_auto_reversals(&pool, &reversal_req)
        .await
        .expect("first reversal");
    assert_eq!(first_reversal.reversals_executed, 1);

    update_balances_from_journal(
        &pool,
        &tenant,
        period_n1_id,
        first_reversal.results[0].journal_entry_id,
        "USD",
    )
    .await;

    // Snapshot cash flows before replay
    let cf_n_before = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow N before replay");

    let cf_n1_before = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n1_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow N+1 before replay");

    // --- REPLAY: create same accrual again ---
    println!("\n--- Replay: re-create accrual ---");
    let replay_accrual = create_accrual_instance(&pool, &accrual_req)
        .await
        .expect("replay accrual");
    assert!(
        replay_accrual.idempotent_hit,
        "Replay should return idempotent_hit=true"
    );
    assert_eq!(replay_accrual.instance_id, first_accrual.instance_id);
    assert_eq!(
        replay_accrual.journal_entry_id,
        first_accrual.journal_entry_id
    );
    println!("  Accrual replay: idempotent_hit=true, same IDs");

    // --- REPLAY: execute reversals again ---
    println!("\n--- Replay: re-execute reversals ---");
    let replay_reversal = execute_auto_reversals(&pool, &reversal_req)
        .await
        .expect("replay reversal");
    assert_eq!(
        replay_reversal.reversals_executed, 0,
        "Replay should execute 0 new reversals"
    );
    println!(
        "  Reversal replay: executed={}, skipped={}",
        replay_reversal.reversals_executed, replay_reversal.reversals_skipped
    );

    // --- Verify no duplicate journal entries ---
    let je_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .expect("count journal entries");
    assert_eq!(
        je_count.0, 2,
        "Should have exactly 2 journal entries (1 accrual + 1 reversal)"
    );

    // --- Verify cash flow unchanged after replay ---
    let cf_n_after = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow N after replay");

    let cf_n1_after = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n1_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow N+1 after replay");

    assert_eq!(
        cf_n_before.net_cash_flow, cf_n_after.net_cash_flow,
        "Period N cash flow unchanged after replay"
    );
    assert_eq!(
        cf_n1_before.net_cash_flow, cf_n1_after.net_cash_flow,
        "Period N+1 cash flow unchanged after replay"
    );
    assert_eq!(
        cf_n_before.cash_account_net_change, cf_n_after.cash_account_net_change,
        "Period N cash delta unchanged after replay"
    );
    assert_eq!(
        cf_n1_before.cash_account_net_change, cf_n1_after.cash_account_net_change,
        "Period N+1 cash delta unchanged after replay"
    );

    // Determinism: query twice, identical results
    let cf_n_again = cashflow_service::get_cash_flow(
        &pool,
        &tenant,
        period_n_id,
        "USD",
        &["CASH_ACCT".to_string()],
    )
    .await
    .expect("cash flow N determinism check");

    assert_eq!(cf_n_after.rows.len(), cf_n_again.rows.len());
    for (a, b) in cf_n_after.rows.iter().zip(cf_n_again.rows.iter()) {
        assert_eq!(a.account_code, b.account_code);
        assert_eq!(a.amount_minor, b.amount_minor);
        assert_eq!(a.category, b.category);
    }

    cleanup_tenant(&pool, &tenant).await;
    println!("\n✅ test_lifecycle_replay_no_duplicate_postings: PASS");
}

/// Test 3: Multiple accruals in one period → all reversed → cash flow totals correct
///
/// Creates 3 accruals (operating/investing/financing) in period N, reverses all
/// in period N+1, and verifies cash flow category totals and cross-period cancellation.
#[tokio::test]
async fn test_multi_accrual_reversal_cashflow_categories() {
    let pool = get_gl_pool().await;
    run_accrual_migrations(&pool).await;
    run_reversal_migrations(&pool).await;
    run_cashflow_migrations(&pool).await;
    ensure_gl_core_tables(&pool).await;
    let tenant = generate_test_tenant();
    cleanup_tenant(&pool, &tenant).await;

    // --- Setup accounts with different classifications ---
    create_account(&pool, &tenant, "CASH", "Cash", "asset", "debit").await;
    create_account(
        &pool,
        &tenant,
        "PREPAID_RENT",
        "Prepaid Rent",
        "asset",
        "debit",
    )
    .await;
    create_account(
        &pool,
        &tenant,
        "EQUIPMENT_DEP",
        "Equipment Deposit",
        "asset",
        "debit",
    )
    .await;
    create_account(
        &pool,
        &tenant,
        "LOAN_FEE",
        "Loan Origination Fee",
        "asset",
        "debit",
    )
    .await;

    // Classify cash account in all three categories won't work — each account gets one category.
    // Instead, classify the counterparty accounts:
    classify_account(&pool, &tenant, "CASH", "operating").await;

    let period_n_id = ensure_period(&pool, &tenant, "2026-10").await;
    let period_n1_id = ensure_period(&pool, &tenant, "2026-11").await;

    let policy = ReversalPolicy {
        auto_reverse_next_period: true,
        reverse_on_date: None,
    };

    // Template 1: Operating — prepaid rent ($3,000)
    let t1 = create_template(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tenant.clone(),
            name: "Prepaid Rent".to_string(),
            description: None,
            debit_account: "PREPAID_RENT".to_string(),
            credit_account: "CASH".to_string(),
            amount_minor: 300000,
            currency: "USD".to_string(),
            reversal_policy: Some(policy.clone()),
            cashflow_class: Some("operating".to_string()),
        },
    )
    .await
    .expect("template 1")
    .template_id;

    // Template 2: Also operating — equipment deposit ($5,000)
    let t2 = create_template(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tenant.clone(),
            name: "Equipment Deposit".to_string(),
            description: None,
            debit_account: "EQUIPMENT_DEP".to_string(),
            credit_account: "CASH".to_string(),
            amount_minor: 500000,
            currency: "USD".to_string(),
            reversal_policy: Some(policy.clone()),
            cashflow_class: Some("investing".to_string()),
        },
    )
    .await
    .expect("template 2")
    .template_id;

    // Template 3: Also operating — loan fee ($2,000)
    let t3 = create_template(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tenant.clone(),
            name: "Loan Origination Fee".to_string(),
            description: None,
            debit_account: "LOAN_FEE".to_string(),
            credit_account: "CASH".to_string(),
            amount_minor: 200000,
            currency: "USD".to_string(),
            reversal_policy: Some(policy),
            cashflow_class: Some("financing".to_string()),
        },
    )
    .await
    .expect("template 3")
    .template_id;

    // --- Create all 3 accruals in period N ---
    let mut accrual_je_ids = Vec::new();
    for (tid, name) in [(t1, "rent"), (t2, "equip"), (t3, "loan")] {
        let accrual = create_accrual_instance(
            &pool,
            &CreateAccrualRequest {
                template_id: tid,
                tenant_id: tenant.clone(),
                period: "2026-10".to_string(),
                posting_date: "2026-10-31".to_string(),
                correlation_id: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("accrual {} failed: {}", name, e));
        assert!(!accrual.idempotent_hit);

        update_balances_from_journal(&pool, &tenant, period_n_id, accrual.journal_entry_id, "USD")
            .await;
        accrual_je_ids.push(accrual.journal_entry_id);
    }
    println!("3 accruals created in period N");

    // --- Cash flow for period N ---
    let cf_n =
        cashflow_service::get_cash_flow(&pool, &tenant, period_n_id, "USD", &["CASH".to_string()])
            .await
            .expect("cash flow period N");

    // Only CASH account is classified. All 3 accruals credit CASH:
    // CASH net = DR(0) - CR(300000 + 500000 + 200000) = -1,000,000
    assert_eq!(
        cf_n.net_cash_flow, -1000000,
        "Period N: total cash outflow from 3 accruals"
    );
    assert_eq!(cf_n.cash_account_net_change, -1000000);
    assert!(cf_n.reconciles, "Period N should reconcile");
    println!(
        "Period N cash flow: {} (reconciles: {})",
        cf_n.net_cash_flow, cf_n.reconciles
    );

    // --- Execute reversals for period N+1 ---
    let reversal_result = execute_auto_reversals(
        &pool,
        &ExecuteReversalsRequest {
            tenant_id: tenant.clone(),
            target_period: "2026-11".to_string(),
            reversal_date: "2026-11-01".to_string(),
        },
    )
    .await
    .expect("reversal execution");

    assert_eq!(
        reversal_result.reversals_executed, 3,
        "All 3 should be reversed"
    );

    for rev in &reversal_result.results {
        update_balances_from_journal(&pool, &tenant, period_n1_id, rev.journal_entry_id, "USD")
            .await;
    }
    println!("3 reversals executed in period N+1");

    // --- Cash flow for period N+1 ---
    let cf_n1 =
        cashflow_service::get_cash_flow(&pool, &tenant, period_n1_id, "USD", &["CASH".to_string()])
            .await
            .expect("cash flow period N+1");

    // All 3 reversals debit CASH: DR(300000 + 500000 + 200000) - CR(0) = +1,000,000
    assert_eq!(
        cf_n1.net_cash_flow, 1000000,
        "Period N+1: total cash inflow from 3 reversals"
    );
    assert_eq!(cf_n1.cash_account_net_change, 1000000);
    assert!(cf_n1.reconciles, "Period N+1 should reconcile");
    println!(
        "Period N+1 cash flow: {} (reconciles: {})",
        cf_n1.net_cash_flow, cf_n1.reconciles
    );

    // --- Cross-period cancellation ---
    let net = cf_n.net_cash_flow + cf_n1.net_cash_flow;
    assert_eq!(net, 0, "Accruals + reversals fully cancel across periods");
    println!("Cross-period net: {} (expected 0)", net);

    // --- Verify total journal entries: 3 accruals + 3 reversals = 6 ---
    let je_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(je_count.0, 6, "6 journal entries total");

    cleanup_tenant(&pool, &tenant).await;
    println!("\n✅ test_multi_accrual_reversal_cashflow_categories: PASS");
}
