/// Reporting GL Statements E2E Tests (Phase 27, bd-qgx7)
///
/// Verifies the full pipeline: GL posting events → trial balance cache →
/// P&L and Balance Sheet statement endpoints.
///
/// Acceptance criteria:
/// 1. E2E passes with real services and deterministic time
/// 2. Trial balance reconciles and statements return correct totals
/// 3. Replay/backfill produces identical caches
///
/// Run with: cargo test -p e2e-tests -- reporting_gl --nocapture
mod common;

use chrono::NaiveDate;
use common::get_reporting_pool;
use event_bus::BusMessage;
use reporting::domain::statements::{balance_sheet, pl};
use reporting::ingest::gl::TrialBalanceHandler;
use reporting::ingest::IngestConsumer;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

const SUBJECT: &str = "gl.events.posting.requested";

/// Build a GL posting EventEnvelope payload.
fn make_posting_envelope(
    tenant_id: &str,
    event_id: &str,
    posting_date: &str,
    currency: &str,
    lines: &[(&str, f64, f64)],
) -> Vec<u8> {
    let line_values: Vec<serde_json::Value> = lines
        .iter()
        .map(|(acct, dr, cr)| {
            serde_json::json!({
                "account_ref": acct,
                "debit": dr,
                "credit": cr
            })
        })
        .collect();

    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "data": {
            "posting_date": posting_date,
            "currency": currency,
            "source_doc_type": "E2E_TEST",
            "source_doc_id": format!("e2e-{}", event_id),
            "description": "E2E GL statement test",
            "lines": line_values
        }
    }))
    .unwrap()
}

/// Create a reporting DB pool with migrations applied.
async fn setup_reporting_pool() -> PgPool {
    let pool = get_reporting_pool().await;
    sqlx::migrate!("../modules/reporting/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run reporting migrations");
    pool
}

/// Clean up test data for a tenant from all reporting cache tables.
async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    for table in &[
        "rpt_trial_balance_cache",
        "rpt_ingestion_checkpoints",
        "rpt_statement_cache",
    ] {
        sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

/// Ingest a batch of GL posting events through the TrialBalanceHandler.
async fn ingest_postings(
    pool: &PgPool,
    tenant_id: &str,
    consumer_prefix: &str,
    postings: &[(&str, &str, &str, Vec<(&str, f64, f64)>)],
) {
    let handler = Arc::new(TrialBalanceHandler);
    for (i, (event_id, date, currency, lines)) in postings.iter().enumerate() {
        let consumer = IngestConsumer::new(
            format!("{}-{}", consumer_prefix, i),
            pool.clone(),
            handler.clone(),
        );
        let msg = BusMessage::new(
            SUBJECT.to_string(),
            make_posting_envelope(tenant_id, event_id, date, currency, lines),
        );
        let processed = consumer
            .process_message(&msg)
            .await
            .expect("ingestion failed");
        assert!(processed, "event {} must be processed", event_id);
    }
}

/// Query trial balance totals (sum of debits/credits) for reconciliation.
async fn trial_balance_totals(pool: &PgPool, tenant_id: &str, currency: &str) -> (i64, i64) {
    sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT COALESCE(SUM(debit_minor), 0)::BIGINT,
               COALESCE(SUM(credit_minor), 0)::BIGINT
        FROM rpt_trial_balance_cache
        WHERE tenant_id = $1 AND currency = $2
        "#,
    )
    .bind(tenant_id)
    .bind(currency)
    .fetch_one(pool)
    .await
    .expect("trial balance totals query failed")
}

// ============================================================================
// Test 1: Full pipeline — GL postings → trial balance → P&L → Balance Sheet
// ============================================================================

/// Posts a realistic set of balanced GL entries covering revenue, COGS,
/// expenses, assets, and liabilities. Then verifies:
///   - Trial balance cache rows exist and reconcile
///   - P&L returns correct revenue/expense totals and net income
///   - Balance Sheet returns correct asset/liability amounts
#[tokio::test]
async fn test_reporting_gl_full_pipeline() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-rpt-gl-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    // --- Post balanced GL entries ---
    // Entry 1: Invoice — DR AR(1100) 5000.00, CR Revenue(4000) 5000.00
    // Entry 2: Payment — DR Cash(1000) 5000.00, CR AR(1100) 5000.00
    // Entry 3: COGS — DR COGS(5000) 2000.00, CR Inventory(1200) 2000.00
    // Entry 4: Expense — DR Rent(6000) 1000.00, CR Cash(1000) 1000.00
    // Entry 5: Liability — DR Cash(1000) 10000.00, CR Loan(2000) 10000.00
    let postings: Vec<(&str, &str, &str, Vec<(&str, f64, f64)>)> = vec![
        (
            "evt-pipe-001",
            "2026-03-15",
            "USD",
            vec![("1100", 5000.00, 0.0), ("4000", 0.0, 5000.00)],
        ),
        (
            "evt-pipe-002",
            "2026-03-16",
            "USD",
            vec![("1000", 5000.00, 0.0), ("1100", 0.0, 5000.00)],
        ),
        (
            "evt-pipe-003",
            "2026-03-17",
            "USD",
            vec![("5000", 2000.00, 0.0), ("1200", 0.0, 2000.00)],
        ),
        (
            "evt-pipe-004",
            "2026-03-20",
            "USD",
            vec![("6000", 1000.00, 0.0), ("1000", 0.0, 1000.00)],
        ),
        (
            "evt-pipe-005",
            "2026-03-01",
            "USD",
            vec![("1000", 10000.00, 0.0), ("2000", 0.0, 10000.00)],
        ),
    ];

    ingest_postings(&pool, &tenant_id, "e2e-pipe", &postings).await;

    // --- Trial balance reconciliation ---
    let (total_debit, total_credit) = trial_balance_totals(&pool, &tenant_id, "USD").await;
    assert_eq!(
        total_debit, total_credit,
        "Trial balance must reconcile: debits ({}) == credits ({})",
        total_debit, total_credit
    );
    assert!(total_debit > 0, "must have non-zero postings");
    println!(
        "Trial balance reconciles: debits = credits = {}",
        total_debit
    );

    // --- P&L statement ---
    let from = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let pl_stmt = pl::compute_pl(&pool, &tenant_id, from, to)
        .await
        .expect("P&L computation failed");

    let rev_section = pl_stmt
        .sections
        .iter()
        .find(|s| s.section == "revenue")
        .unwrap();
    let rev_usd = rev_section
        .total_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    // Revenue 4000: credit 500000, debit 0 → amount = 500000
    assert_eq!(
        rev_usd, 500000,
        "Revenue should be $5,000.00 (500000 minor)"
    );

    let cogs_section = pl_stmt
        .sections
        .iter()
        .find(|s| s.section == "cogs")
        .unwrap();
    let cogs_usd = cogs_section
        .total_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    // COGS 5000: debit 200000, credit 0 → amount = 200000
    assert_eq!(cogs_usd, 200000, "COGS should be $2,000.00 (200000 minor)");

    let exp_section = pl_stmt
        .sections
        .iter()
        .find(|s| s.section == "expenses")
        .unwrap();
    let exp_usd = exp_section
        .total_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    // Expenses 6000: debit 100000, credit 0 → amount = 100000
    assert_eq!(
        exp_usd, 100000,
        "Expenses should be $1,000.00 (100000 minor)"
    );

    // Net income = revenue - cogs - expenses = 500000 - 200000 - 100000 = 200000
    let net_income = pl_stmt
        .net_income_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    assert_eq!(
        net_income, 200000,
        "Net income should be $2,000.00 (200000 minor)"
    );
    println!(
        "P&L: revenue={}, cogs={}, expenses={}, net_income={}",
        rev_usd, cogs_usd, exp_usd, net_income
    );

    // --- Balance Sheet ---
    let as_of = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let bs = balance_sheet::compute_balance_sheet(&pool, &tenant_id, as_of)
        .await
        .expect("Balance sheet computation failed");

    let assets = bs.sections.iter().find(|s| s.section == "assets").unwrap();
    let assets_usd = assets.total_by_currency.get("USD").copied().unwrap_or(0);
    // Assets:
    //   1000 Cash: DR 5000+10000=15000, CR 1000=1000 → net 14000.00 = 1400000
    //   1100 AR: DR 5000, CR 5000 → net 0
    //   1200 Inventory: DR 0, CR 2000 → net -200000 (debit-normal: debit-credit)
    // Total = 1400000 + 0 + (-200000) = 1200000
    assert_eq!(assets_usd, 1200000, "Total assets should be $12,000.00");

    let liabilities = bs
        .sections
        .iter()
        .find(|s| s.section == "liabilities")
        .unwrap();
    let liabilities_usd = liabilities
        .total_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    // Liabilities:
    //   2000 Loan: DR 0, CR 10000 → credit-normal: credit-debit = 1000000
    assert_eq!(
        liabilities_usd, 1000000,
        "Total liabilities should be $10,000.00"
    );

    println!(
        "Balance Sheet: assets={}, liabilities={}",
        assets_usd, liabilities_usd
    );

    println!("\nPASS: Full GL → trial balance → P&L/BS pipeline verified");
    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Replay/backfill produces identical caches (idempotency)
// ============================================================================

/// Ingests events, records cache state, wipes checkpoints (simulating a
/// backfill scenario), re-ingests the same events, and asserts the cache
/// is identical.
#[tokio::test]
async fn test_reporting_gl_replay_produces_identical_cache() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-rpt-replay-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let postings: Vec<(&str, &str, &str, Vec<(&str, f64, f64)>)> = vec![
        (
            "evt-replay-001",
            "2026-04-01",
            "USD",
            vec![("1100", 3000.00, 0.0), ("4000", 0.0, 3000.00)],
        ),
        (
            "evt-replay-002",
            "2026-04-05",
            "USD",
            vec![("5000", 1500.00, 0.0), ("1200", 0.0, 1500.00)],
        ),
        (
            "evt-replay-003",
            "2026-04-10",
            "EUR",
            vec![("1000", 2000.00, 0.0), ("2000", 0.0, 2000.00)],
        ),
    ];

    // First ingestion
    ingest_postings(&pool, &tenant_id, "e2e-replay-a", &postings).await;

    // Snapshot the cache state
    let snapshot_before: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_code, currency, debit_minor, credit_minor, net_minor
        FROM rpt_trial_balance_cache
        WHERE tenant_id = $1
        ORDER BY account_code, currency
        "#,
    )
    .bind(&tenant_id)
    .fetch_all(&pool)
    .await
    .expect("snapshot query failed");
    assert!(
        !snapshot_before.is_empty(),
        "cache must have rows after first ingestion"
    );

    // Wipe checkpoints to simulate a backfill from scratch, then wipe cache
    sqlx::query("DELETE FROM rpt_ingestion_checkpoints WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .expect("wipe checkpoints");
    sqlx::query("DELETE FROM rpt_trial_balance_cache WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .expect("wipe cache");

    // Re-ingest with different consumer names (simulating fresh consumers)
    ingest_postings(&pool, &tenant_id, "e2e-replay-b", &postings).await;

    // Snapshot again
    let snapshot_after: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_code, currency, debit_minor, credit_minor, net_minor
        FROM rpt_trial_balance_cache
        WHERE tenant_id = $1
        ORDER BY account_code, currency
        "#,
    )
    .bind(&tenant_id)
    .fetch_all(&pool)
    .await
    .expect("snapshot after replay query failed");

    // Assert identical
    assert_eq!(
        snapshot_before.len(),
        snapshot_after.len(),
        "row count must match: before={}, after={}",
        snapshot_before.len(),
        snapshot_after.len()
    );
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        assert_eq!(
            before, after,
            "cache row mismatch after replay: before={:?}, after={:?}",
            before, after
        );
    }

    // Verify reconciliation holds after replay
    let (td, tc) = trial_balance_totals(&pool, &tenant_id, "USD").await;
    assert_eq!(td, tc, "USD trial balance must reconcile after replay");
    let (td_eur, tc_eur) = trial_balance_totals(&pool, &tenant_id, "EUR").await;
    assert_eq!(
        td_eur, tc_eur,
        "EUR trial balance must reconcile after replay"
    );

    println!(
        "PASS: Replay/backfill produces identical cache ({} rows)",
        snapshot_after.len()
    );
    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Multi-currency P&L isolation
// ============================================================================

/// Posts revenue in USD and EUR on the same date and verifies P&L
/// reports each currency independently with no cross-contamination.
#[tokio::test]
async fn test_reporting_gl_multicurrency_pl() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-rpt-multicur-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let postings: Vec<(&str, &str, &str, Vec<(&str, f64, f64)>)> = vec![
        (
            "evt-mcur-001",
            "2026-05-15",
            "USD",
            vec![("1100", 8000.00, 0.0), ("4000", 0.0, 8000.00)],
        ),
        (
            "evt-mcur-002",
            "2026-05-15",
            "EUR",
            vec![("1100", 6000.00, 0.0), ("4000", 0.0, 6000.00)],
        ),
        (
            "evt-mcur-003",
            "2026-05-20",
            "USD",
            vec![("5000", 3000.00, 0.0), ("1200", 0.0, 3000.00)],
        ),
    ];

    ingest_postings(&pool, &tenant_id, "e2e-mcur", &postings).await;

    let from = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
    let stmt = pl::compute_pl(&pool, &tenant_id, from, to)
        .await
        .expect("P&L failed");

    let rev = stmt
        .sections
        .iter()
        .find(|s| s.section == "revenue")
        .unwrap();
    assert_eq!(
        rev.total_by_currency.get("USD").copied().unwrap_or(0),
        800000
    );
    assert_eq!(
        rev.total_by_currency.get("EUR").copied().unwrap_or(0),
        600000
    );

    let cogs = stmt.sections.iter().find(|s| s.section == "cogs").unwrap();
    assert_eq!(
        cogs.total_by_currency.get("USD").copied().unwrap_or(0),
        300000
    );
    assert_eq!(cogs.total_by_currency.get("EUR").copied().unwrap_or(0), 0);

    // Net income: USD = 800000 - 300000 = 500000, EUR = 600000
    assert_eq!(
        stmt.net_income_by_currency.get("USD").copied().unwrap_or(0),
        500000
    );
    assert_eq!(
        stmt.net_income_by_currency.get("EUR").copied().unwrap_or(0),
        600000
    );

    println!("PASS: Multi-currency P&L isolation verified");
    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Deterministic — repeated queries return identical results
// ============================================================================

/// Runs P&L and Balance Sheet twice and asserts output is bit-identical.
#[tokio::test]
async fn test_reporting_gl_deterministic_queries() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-rpt-det-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let postings: Vec<(&str, &str, &str, Vec<(&str, f64, f64)>)> = vec![
        (
            "evt-det-001",
            "2026-06-10",
            "USD",
            vec![("1000", 7500.00, 0.0), ("4000", 0.0, 7500.00)],
        ),
        (
            "evt-det-002",
            "2026-06-15",
            "USD",
            vec![("6000", 2500.00, 0.0), ("1000", 0.0, 2500.00)],
        ),
    ];

    ingest_postings(&pool, &tenant_id, "e2e-det", &postings).await;

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
    let as_of = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();

    // Query twice
    let pl1 = pl::compute_pl(&pool, &tenant_id, from, to).await.unwrap();
    let pl2 = pl::compute_pl(&pool, &tenant_id, from, to).await.unwrap();

    let bs1 = balance_sheet::compute_balance_sheet(&pool, &tenant_id, as_of)
        .await
        .unwrap();
    let bs2 = balance_sheet::compute_balance_sheet(&pool, &tenant_id, as_of)
        .await
        .unwrap();

    // P&L must be identical
    assert_eq!(pl1.net_income_by_currency, pl2.net_income_by_currency);
    for (s1, s2) in pl1.sections.iter().zip(pl2.sections.iter()) {
        assert_eq!(s1.section, s2.section);
        assert_eq!(s1.total_by_currency, s2.total_by_currency);
        assert_eq!(s1.accounts.len(), s2.accounts.len());
        for (a1, a2) in s1.accounts.iter().zip(s2.accounts.iter()) {
            assert_eq!(a1.account_code, a2.account_code);
            assert_eq!(a1.amount_minor, a2.amount_minor);
        }
    }

    // Balance Sheet must be identical
    for (s1, s2) in bs1.sections.iter().zip(bs2.sections.iter()) {
        assert_eq!(s1.section, s2.section);
        assert_eq!(s1.total_by_currency, s2.total_by_currency);
        assert_eq!(s1.accounts.len(), s2.accounts.len());
        for (a1, a2) in s1.accounts.iter().zip(s2.accounts.iter()) {
            assert_eq!(a1.account_code, a2.account_code);
            assert_eq!(a1.amount_minor, a2.amount_minor);
        }
    }

    println!("PASS: P&L and Balance Sheet are deterministic across queries");
    cleanup_tenant(&pool, &tenant_id).await;
}
