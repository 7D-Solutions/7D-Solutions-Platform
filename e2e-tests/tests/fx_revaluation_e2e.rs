/// FX Revaluation at Period Close E2E Tests (Phase 23a, bd-1yu)
///
/// Verifies:
/// 1. Period close triggers FX revaluation for foreign-currency balances
/// 2. Revaluation journal entry is balanced (debits == credits)
/// 3. Correct adjustment amounts based on opening vs closing FX rates
/// 4. Revaluation is idempotent (closing twice doesn't double-post)
/// 5. No revaluation when no foreign-currency balances exist
///
/// Run with: cargo test -p e2e-tests fx_revaluation_e2e -- --nocapture
mod common;

use chrono::NaiveDate;
use common::get_gl_pool;
use gl_rs::services::period_close_service::close_period;
use sqlx::{PgPool, Row};
use uuid::Uuid;

// ============================================================================
// Setup helpers
// ============================================================================

/// Clean up all test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    // Clean processed_events for journal entries belonging to this tenant
    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM fx_rates WHERE tenant_id = $1")
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

/// Create an accounting period and return its ID.
async fn create_period(pool: &PgPool, tenant_id: &str, start: NaiveDate, end: NaiveDate) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, false, NOW())
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(start)
    .bind(end)
    .execute(pool)
    .await
    .expect("create period");
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

/// Insert an FX rate directly.
async fn insert_fx_rate(
    pool: &PgPool,
    tenant_id: &str,
    base: &str,
    quote: &str,
    rate: f64,
    effective_at: NaiveDate,
) {
    let effective_ts = effective_at.and_hms_opt(0, 0, 0).unwrap().and_utc();
    sqlx::query(
        r#"
        INSERT INTO fx_rates (id, tenant_id, base_currency, quote_currency, rate, inverse_rate,
                              effective_at, source, idempotency_key, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'test', $8, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(base)
    .bind(quote)
    .bind(rate)
    .bind(1.0 / rate)
    .bind(effective_ts)
    .bind(format!("fx-test-{}", Uuid::new_v4()))
    .execute(pool)
    .await
    .expect("insert fx rate");
}

/// Post a balanced journal entry and create corresponding account_balances.
///
/// Creates a simple 2-line entry: DR `debit_account`, CR `credit_account`.
async fn post_journal_with_balances(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,
    debit_account: &str,
    credit_account: &str,
    amount_minor: i64,
    posting_date: NaiveDate,
) -> Uuid {
    let entry_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let posted_at = posting_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    // Insert journal entry header
    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject,
                                     posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', $3, 'test.posting', $4, $5, 'E2E test entry', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(event_id)
    .bind(posted_at)
    .bind(currency)
    .execute(pool)
    .await
    .expect("insert journal entry");

    // Insert balanced lines
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES
            ($1, $2, 1, $3, $4, 0, 'Debit line'),
            ($5, $2, 2, $6, 0, $4, 'Credit line')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(debit_account)
    .bind(amount_minor)
    .bind(Uuid::new_v4())
    .bind(credit_account)
    .execute(pool)
    .await
    .expect("insert journal lines");

    // Mark event processed
    sqlx::query(
        "INSERT INTO processed_events (event_id, event_type, processor) VALUES ($1, 'test.posting', 'test')",
    )
    .bind(event_id)
    .execute(pool)
    .await
    .expect("insert processed event");

    // Upsert account balances (debit side)
    sqlx::query(
        r#"
        INSERT INTO account_balances (id, tenant_id, period_id, account_code, currency,
                                      debit_total_minor, credit_total_minor, net_balance_minor,
                                      last_journal_entry_id, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, 0, $6, $7, NOW(), NOW())
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            debit_total_minor = account_balances.debit_total_minor + $6,
            net_balance_minor = account_balances.net_balance_minor + $6,
            last_journal_entry_id = $7,
            updated_at = NOW()
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(debit_account)
    .bind(currency)
    .bind(amount_minor)
    .bind(entry_id)
    .execute(pool)
    .await
    .expect("upsert debit balance");

    // Upsert account balances (credit side)
    sqlx::query(
        r#"
        INSERT INTO account_balances (id, tenant_id, period_id, account_code, currency,
                                      debit_total_minor, credit_total_minor, net_balance_minor,
                                      last_journal_entry_id, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, 0, $6, -$6, $7, NOW(), NOW())
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            credit_total_minor = account_balances.credit_total_minor + $6,
            net_balance_minor = account_balances.net_balance_minor - $6,
            last_journal_entry_id = $7,
            updated_at = NOW()
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(credit_account)
    .bind(currency)
    .bind(amount_minor)
    .bind(entry_id)
    .execute(pool)
    .await
    .expect("upsert credit balance");

    entry_id
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Period close produces a balanced revaluation journal entry
/// when foreign-currency balances exist with rate changes.
#[tokio::test]
async fn test_fx_revaluation_at_period_close() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-fxreval-{}", Uuid::new_v4());

    cleanup_tenant(&pool, &tenant_id).await;

    // Setup accounts
    create_account(
        &pool,
        &tenant_id,
        "1100",
        "Accounts Receivable",
        "asset",
        "debit",
    )
    .await;
    create_account(&pool, &tenant_id, "4000", "Revenue", "revenue", "credit").await;
    create_account(
        &pool,
        &tenant_id,
        "7100",
        "Unrealized FX Gain/Loss",
        "expense",
        "debit",
    )
    .await;

    // Create period: January 2025
    let period_start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2025, 1, 31).unwrap();
    let period_id = create_period(&pool, &tenant_id, period_start, period_end).await;

    // Post EUR journal: DR 1100 EUR 1000.00, CR 4000 EUR 1000.00
    let _entry_id = post_journal_with_balances(
        &pool,
        &tenant_id,
        period_id,
        "EUR",
        "1100",
        "4000",
        100_000, // 1000.00 in minor units
        NaiveDate::from_ymd_opt(2025, 1, 15).unwrap(),
    )
    .await;

    // Insert FX rates:
    //   Opening: EUR/USD = 1.08 effective Jan 1
    //   Closing: EUR/USD = 1.10 effective Jan 31
    insert_fx_rate(&pool, &tenant_id, "EUR", "USD", 1.08, period_start).await;
    insert_fx_rate(&pool, &tenant_id, "EUR", "USD", 1.10, period_end).await;

    // Close the period — this triggers FX revaluation
    let result = close_period(
        &pool,
        &tenant_id,
        period_id,
        "e2e-test",
        Some("FX revaluation test"),
        false,
        "USD",
    )
    .await
    .expect("close_period should succeed");

    assert!(result.success, "Period close should succeed");
    println!(
        "Period closed successfully with hash: {:?}",
        result.close_status
    );

    // Verify revaluation journal entry was created
    let reval_entry = sqlx::query(
        r#"
        SELECT id, currency, description, source_subject
        FROM journal_entries
        WHERE tenant_id = $1
          AND source_subject = 'gl.revaluation.period_close'
        "#,
    )
    .bind(&tenant_id)
    .fetch_optional(&pool)
    .await
    .expect("query reval entry");

    let reval_entry = reval_entry.expect("Revaluation journal entry should exist");
    let reval_entry_id: Uuid = reval_entry.get("id");
    let currency: String = reval_entry.get("currency");
    let description: String = reval_entry.get("description");

    assert_eq!(
        currency, "USD",
        "Revaluation should be in reporting currency"
    );
    assert_eq!(description, "Unrealized FX revaluation at period close");
    println!("Revaluation entry ID: {}", reval_entry_id);

    // Verify the entry is balanced
    let balance_row = sqlx::query(
        r#"
        SELECT
            COALESCE(SUM(debit_minor), 0)::BIGINT as total_debits,
            COALESCE(SUM(credit_minor), 0)::BIGINT as total_credits,
            COUNT(*)::BIGINT as line_count
        FROM journal_lines
        WHERE journal_entry_id = $1
        "#,
    )
    .bind(reval_entry_id)
    .fetch_one(&pool)
    .await
    .expect("query journal lines");

    let total_debits: i64 = balance_row.get("total_debits");
    let total_credits: i64 = balance_row.get("total_credits");
    let line_count: i64 = balance_row.get("line_count");

    assert_eq!(
        total_debits, total_credits,
        "Revaluation entry must be balanced: debits={} credits={}",
        total_debits, total_credits
    );
    assert!(
        line_count >= 2,
        "Should have at least 2 lines, got {}",
        line_count
    );
    println!(
        "Revaluation balanced: debits={} credits={} lines={}",
        total_debits, total_credits, line_count
    );

    // Verify adjustment amounts:
    //   1100 EUR: net_balance=100000, opening=100000*1.08=108000, closing=100000*1.10=110000
    //     adjustment = +2000 (gain) → DR 1100 2000, CR 7100 2000
    //   4000 EUR: net_balance=-100000, opening=-100000*1.08=-108000, closing=-100000*1.10=-110000
    //     adjustment = -2000 (loss) → DR 7100 2000, CR 4000 2000
    // Total: DR (1100: 2000 + 7100: 2000) = 4000, CR (7100: 2000 + 4000: 2000) = 4000
    assert_eq!(
        total_debits, 4000,
        "Expected total debits of 4000 minor units"
    );

    // Verify individual lines
    let lines: Vec<_> = sqlx::query(
        r#"
        SELECT account_ref, debit_minor, credit_minor, memo
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(reval_entry_id)
    .fetch_all(&pool)
    .await
    .expect("fetch lines");

    println!("Revaluation journal lines:");
    for line in &lines {
        let account: String = line.get("account_ref");
        let debit: i64 = line.get("debit_minor");
        let credit: i64 = line.get("credit_minor");
        let memo: Option<String> = line.get("memo");
        println!(
            "  {} DR={} CR={} {}",
            account,
            debit,
            credit,
            memo.unwrap_or_default()
        );
    }

    println!("PASS: FX revaluation at period close");
}

/// Test 2: Closing a period twice does NOT create duplicate revaluation entries
/// (idempotency via closed_at check in close_period).
#[tokio::test]
async fn test_fx_revaluation_idempotent() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-fxreval-idem-{}", Uuid::new_v4());

    cleanup_tenant(&pool, &tenant_id).await;

    // Setup
    create_account(&pool, &tenant_id, "1100", "AR", "asset", "debit").await;
    create_account(&pool, &tenant_id, "4000", "Revenue", "revenue", "credit").await;
    create_account(
        &pool,
        &tenant_id,
        "7100",
        "FX Gain/Loss",
        "expense",
        "debit",
    )
    .await;

    let period_start = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2025, 2, 28).unwrap();
    let period_id = create_period(&pool, &tenant_id, period_start, period_end).await;

    post_journal_with_balances(
        &pool,
        &tenant_id,
        period_id,
        "EUR",
        "1100",
        "4000",
        50_000,
        NaiveDate::from_ymd_opt(2025, 2, 15).unwrap(),
    )
    .await;

    insert_fx_rate(&pool, &tenant_id, "EUR", "USD", 1.05, period_start).await;
    insert_fx_rate(&pool, &tenant_id, "EUR", "USD", 1.12, period_end).await;

    // First close
    let result1 = close_period(&pool, &tenant_id, period_id, "user1", None, false, "USD")
        .await
        .expect("first close");
    assert!(result1.success, "First close should succeed");

    // Count revaluation entries after first close
    let count_after_first: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_subject = 'gl.revaluation.period_close'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count reval entries");

    assert_eq!(
        count_after_first, 1,
        "Should have exactly 1 revaluation entry"
    );

    // Second close (idempotent)
    let result2 = close_period(&pool, &tenant_id, period_id, "user2", None, false, "USD")
        .await
        .expect("second close");
    assert!(result2.success, "Second close should succeed (idempotent)");

    // Count should NOT increase
    let count_after_second: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_subject = 'gl.revaluation.period_close'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count reval entries");

    assert_eq!(
        count_after_second, 1,
        "Idempotent close should NOT create duplicate revaluation entries"
    );

    println!("PASS: FX revaluation idempotent");
}

/// Test 3: No revaluation when only reporting-currency balances exist.
#[tokio::test]
async fn test_fx_revaluation_skips_when_no_foreign_balances() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-fxreval-nofx-{}", Uuid::new_v4());

    cleanup_tenant(&pool, &tenant_id).await;

    // Setup USD-only accounts and entry
    create_account(&pool, &tenant_id, "1000", "Cash", "asset", "debit").await;
    create_account(&pool, &tenant_id, "4000", "Revenue", "revenue", "credit").await;
    create_account(
        &pool,
        &tenant_id,
        "7100",
        "FX Gain/Loss",
        "expense",
        "debit",
    )
    .await;

    let period_start = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();
    let period_id = create_period(&pool, &tenant_id, period_start, period_end).await;

    // Post USD entry (not foreign)
    post_journal_with_balances(
        &pool,
        &tenant_id,
        period_id,
        "USD",
        "1000",
        "4000",
        200_000,
        NaiveDate::from_ymd_opt(2025, 3, 10).unwrap(),
    )
    .await;

    // Close period
    let result = close_period(&pool, &tenant_id, period_id, "admin", None, false, "USD")
        .await
        .expect("close period");
    assert!(result.success);

    // No revaluation entry should exist
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_subject = 'gl.revaluation.period_close'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count reval entries");

    assert_eq!(count, 0, "No revaluation entry when only USD balances");

    println!("PASS: No FX revaluation for USD-only balances");
}

/// Test 4: Verify exact adjustment amounts with known rates.
#[tokio::test]
async fn test_fx_revaluation_correct_adjustment_amounts() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-fxreval-amt-{}", Uuid::new_v4());

    cleanup_tenant(&pool, &tenant_id).await;

    create_account(&pool, &tenant_id, "1100", "AR", "asset", "debit").await;
    create_account(&pool, &tenant_id, "4000", "Revenue", "revenue", "credit").await;
    create_account(
        &pool,
        &tenant_id,
        "7100",
        "FX Gain/Loss",
        "expense",
        "debit",
    )
    .await;

    let period_start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2025, 4, 30).unwrap();
    let period_id = create_period(&pool, &tenant_id, period_start, period_end).await;

    // Post GBP journal: 500.00 GBP = 50000 minor
    post_journal_with_balances(
        &pool,
        &tenant_id,
        period_id,
        "GBP",
        "1100",
        "4000",
        50_000,
        NaiveDate::from_ymd_opt(2025, 4, 15).unwrap(),
    )
    .await;

    // GBP/USD rates:
    //   Opening: 1.26 (Apr 1) → 50000 GBP = 63000 USD minor
    //   Closing: 1.30 (Apr 30) → 50000 GBP = 65000 USD minor
    //   Adjustment per account: ±2000 USD minor
    insert_fx_rate(&pool, &tenant_id, "GBP", "USD", 1.26, period_start).await;
    insert_fx_rate(&pool, &tenant_id, "GBP", "USD", 1.30, period_end).await;

    let result = close_period(&pool, &tenant_id, period_id, "admin", None, false, "USD")
        .await
        .expect("close period");
    assert!(result.success);

    // Get revaluation lines
    let reval_entry_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM journal_entries WHERE tenant_id = $1 AND source_subject = 'gl.revaluation.period_close'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("reval entry should exist");

    // Get balance for 1100 in reporting currency
    let lines: Vec<_> = sqlx::query(
        "SELECT account_ref, debit_minor, credit_minor FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(reval_entry_id)
    .fetch_all(&pool)
    .await
    .expect("fetch reval lines");

    // Verify total adjustment per side:
    // 1100: net=+50000 GBP, opening=63000 USD, closing=65000 USD → gain +2000
    //   → DR 1100 2000, CR 7100 2000
    // 4000: net=-50000 GBP, opening=-63000 USD, closing=-65000 USD → loss -2000
    //   → DR 7100 2000, CR 4000 2000
    let total_debits: i64 = lines.iter().map(|r| r.get::<i64, _>("debit_minor")).sum();
    let total_credits: i64 = lines.iter().map(|r| r.get::<i64, _>("credit_minor")).sum();

    assert_eq!(
        total_debits, 4000,
        "Total debits should be 4000 (2000 per account)"
    );
    assert_eq!(
        total_credits, 4000,
        "Total credits should be 4000 (2000 per account)"
    );
    assert_eq!(total_debits, total_credits, "Entry must be balanced");

    // Verify 1100 got a debit (gain increases asset value)
    let ar_debit: i64 = lines
        .iter()
        .filter(|r| r.get::<String, _>("account_ref") == "1100")
        .map(|r| r.get::<i64, _>("debit_minor"))
        .sum();
    assert_eq!(ar_debit, 2000, "1100 should have debit of 2000 (FX gain)");

    // Verify 4000 got a credit (loss decreases revenue liability)
    let rev_credit: i64 = lines
        .iter()
        .filter(|r| r.get::<String, _>("account_ref") == "4000")
        .map(|r| r.get::<i64, _>("credit_minor"))
        .sum();
    assert_eq!(
        rev_credit, 2000,
        "4000 should have credit of 2000 (FX loss)"
    );

    println!("PASS: Correct FX revaluation amounts: GBP +2000 / -2000");
}

/// Test 5: Revaluation entry is included in period close snapshot hash.
#[tokio::test]
async fn test_fx_revaluation_included_in_close_hash() {
    let pool = get_gl_pool().await;
    let tenant_id = format!("test-fxreval-hash-{}", Uuid::new_v4());

    cleanup_tenant(&pool, &tenant_id).await;

    create_account(&pool, &tenant_id, "1100", "AR", "asset", "debit").await;
    create_account(&pool, &tenant_id, "4000", "Revenue", "revenue", "credit").await;
    create_account(
        &pool,
        &tenant_id,
        "7100",
        "FX Gain/Loss",
        "expense",
        "debit",
    )
    .await;

    let period_start = NaiveDate::from_ymd_opt(2025, 5, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
    let period_id = create_period(&pool, &tenant_id, period_start, period_end).await;

    post_journal_with_balances(
        &pool,
        &tenant_id,
        period_id,
        "EUR",
        "1100",
        "4000",
        100_000,
        NaiveDate::from_ymd_opt(2025, 5, 15).unwrap(),
    )
    .await;

    insert_fx_rate(&pool, &tenant_id, "EUR", "USD", 1.08, period_start).await;
    insert_fx_rate(&pool, &tenant_id, "EUR", "USD", 1.12, period_end).await;

    let result = close_period(&pool, &tenant_id, period_id, "admin", None, false, "USD")
        .await
        .expect("close period");
    assert!(result.success);

    // The snapshot should include the revaluation entry's currency (USD for the reval journal)
    let snapshot_currencies: Vec<String> = sqlx::query_scalar(
        "SELECT currency FROM period_summary_snapshots WHERE tenant_id = $1 AND period_id = $2 ORDER BY currency",
    )
    .bind(&tenant_id)
    .bind(period_id)
    .fetch_all(&pool)
    .await
    .expect("fetch snapshots");

    println!("Snapshot currencies: {:?}", snapshot_currencies);
    // Should include both EUR (original entry) and USD (revaluation entry)
    assert!(
        snapshot_currencies.contains(&"EUR".to_string()),
        "Snapshot should include EUR"
    );
    assert!(
        snapshot_currencies.contains(&"USD".to_string()),
        "Snapshot should include USD (from revaluation entry)"
    );

    // Verify the close hash is a valid SHA-256 hex string
    let close_hash: String =
        sqlx::query_scalar("SELECT close_hash FROM accounting_periods WHERE id = $1")
            .bind(period_id)
            .fetch_one(&pool)
            .await
            .expect("fetch close hash");

    assert_eq!(
        close_hash.len(),
        64,
        "Close hash should be SHA-256 (64 hex chars)"
    );
    println!("Close hash includes revaluation: {}", close_hash);

    println!("PASS: Revaluation included in close snapshot hash");
}
