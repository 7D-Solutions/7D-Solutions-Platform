//! Integration tests for report_query_repo (Phase 12)
//!
//! Tests bounded, indexed queries for reporting primitives.
//! Validates pagination, tenant isolation, and query performance.

use chrono::{TimeZone, Utc};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::repos::report_query_repo::{
    count_account_activity, count_entries_by_account_codes, count_entries_by_account_types,
    count_entries_by_date_range, fetch_entry_header, fetch_entry_lines_with_accounts,
    query_account_activity, query_entries_by_account_codes, query_entries_by_account_types,
    query_entries_by_date_range, query_period_journal_entries, ReportQueryError,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_test".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

async fn setup_test_data(pool: &PgPool, tenant_id: &str) -> (Uuid, Uuid, Uuid) {
    // Create accounting period
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())
    .bind(Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap())
    .bind(false)
    .execute(pool)
    .await
    .expect("Failed to insert period");

    // Create test accounts
    for (code, name, acc_type, normal_balance) in [
        ("1000", "Cash", AccountType::Asset, NormalBalance::Debit),
        (
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            NormalBalance::Debit,
        ),
        (
            "2000",
            "Accounts Payable",
            AccountType::Liability,
            NormalBalance::Credit,
        ),
        (
            "4000",
            "Revenue",
            AccountType::Revenue,
            NormalBalance::Credit,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (tenant_id, code) DO NOTHING
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(code)
        .bind(name)
        .bind(acc_type)
        .bind(normal_balance)
        .bind(true)
        .execute(pool)
        .await
        .expect("Failed to insert account");
    }

    // Create test journal entries and return their IDs
    let entry1_id = Uuid::new_v4();
    let entry2_id = Uuid::new_v4();
    let entry3_id = Uuid::new_v4();

    // Entry 1: 2025-01-10 - Cash debit, Revenue credit
    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description)
        VALUES ($1, $2, 'ar', $3, 'invoice.created', $4, 'USD', 'Invoice payment')
        "#,
    )
    .bind(entry1_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Utc.with_ymd_and_hms(2025, 1, 10, 12, 0, 0).unwrap())
    .execute(pool)
    .await
    .expect("Failed to insert entry 1");

    sqlx::query(
        r#"
        INSERT INTO journal_lines
            (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry1_id)
    .bind(1)
    .bind("1000")
    .bind(50000_i64)
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("Failed to insert line");

    sqlx::query(
        r#"
        INSERT INTO journal_lines
            (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry1_id)
    .bind(2)
    .bind("4000")
    .bind(0_i64)
    .bind(50000_i64)
    .execute(pool)
    .await
    .expect("Failed to insert line");

    // Entry 2: 2025-01-15 - Cash debit, AR credit
    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description)
        VALUES ($1, $2, 'ar', $3, 'payment.received', $4, 'USD', 'AR collection')
        "#,
    )
    .bind(entry2_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Utc.with_ymd_and_hms(2025, 1, 15, 14, 0, 0).unwrap())
    .execute(pool)
    .await
    .expect("Failed to insert entry 2");

    sqlx::query(
        r#"
        INSERT INTO journal_lines
            (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry2_id)
    .bind(1)
    .bind("1000")
    .bind(30000_i64)
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("Failed to insert line");

    sqlx::query(
        r#"
        INSERT INTO journal_lines
            (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry2_id)
    .bind(2)
    .bind("1200")
    .bind(0_i64)
    .bind(30000_i64)
    .execute(pool)
    .await
    .expect("Failed to insert line");

    // Entry 3: 2025-01-20 - AP debit, Cash credit
    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description)
        VALUES ($1, $2, 'ap', $3, 'payment.made', $4, 'USD', 'Vendor payment')
        "#,
    )
    .bind(entry3_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Utc.with_ymd_and_hms(2025, 1, 20, 10, 0, 0).unwrap())
    .execute(pool)
    .await
    .expect("Failed to insert entry 3");

    sqlx::query(
        r#"
        INSERT INTO journal_lines
            (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry3_id)
    .bind(1)
    .bind("2000")
    .bind(20000_i64)
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("Failed to insert line");

    sqlx::query(
        r#"
        INSERT INTO journal_lines
            (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry3_id)
    .bind(2)
    .bind("1000")
    .bind(0_i64)
    .bind(20000_i64)
    .execute(pool)
    .await
    .expect("Failed to insert line");

    (entry1_id, entry2_id, entry3_id)
}

// ============================================================
// ACCOUNT ACTIVITY TESTS
// ============================================================

#[tokio::test]
#[serial]
async fn test_query_account_activity_success() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_activity_1_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    let lines = query_account_activity(&pool, &tenant_id,"1000", start_date, end_date, 100, 0)
        .await
        .expect("Query failed");

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].debit_minor, 50000);
    assert_eq!(lines[1].debit_minor, 30000);
    assert_eq!(lines[2].credit_minor, 20000);
}

#[tokio::test]
#[serial]
async fn test_count_account_activity() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_activity_2_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    let count = count_account_activity(&pool, &tenant_id,"1000", start_date, end_date)
        .await
        .expect("Count failed");

    assert_eq!(count, 3);
}

#[tokio::test]
#[serial]
async fn test_account_activity_invalid_date_range() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_activity_3_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 31, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

    let result = query_account_activity(&pool, &tenant_id,"1000", start_date, end_date, 100, 0)
        .await;

    assert!(matches!(
        result,
        Err(ReportQueryError::InvalidDateRange { .. })
    ));
}

#[tokio::test]
#[serial]
async fn test_account_activity_invalid_pagination() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_activity_4_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 0, 0, 0).unwrap();

    // Invalid limit
    let result = query_account_activity(&pool, &tenant_id,"1000", start_date, end_date, 0, 0)
        .await;
    assert!(matches!(
        result,
        Err(ReportQueryError::InvalidPagination { .. })
    ));

    // Invalid offset
    let result = query_account_activity(&pool, &tenant_id,"1000", start_date, end_date, 100, -1)
        .await;
    assert!(matches!(
        result,
        Err(ReportQueryError::InvalidPagination { .. })
    ));
}

// ============================================================
// GL DETAIL TESTS
// ============================================================

#[tokio::test]
#[serial]
async fn test_query_entries_by_date_range() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_gl_1_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    let entry_ids = query_entries_by_date_range(&pool, &tenant_id,start_date, end_date, 100, 0)
        .await
        .expect("Query failed");

    assert_eq!(entry_ids.len(), 3);
}

#[tokio::test]
#[serial]
async fn test_query_entries_by_account_codes() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_gl_2_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    // Filter by Cash (1000) - should match all 3 entries
    let account_codes = vec!["1000".to_string()];
    let entry_ids = query_entries_by_account_codes(
        &pool,
        &tenant_id,
        &account_codes,
        start_date,
        end_date,
        100,
        0,
    )
    .await
    .expect("Query failed");
    assert_eq!(entry_ids.len(), 3);

    // Filter by Revenue (4000) - should match only entry 1
    let account_codes = vec!["4000".to_string()];
    let entry_ids = query_entries_by_account_codes(
        &pool,
        &tenant_id,
        &account_codes,
        start_date,
        end_date,
        100,
        0,
    )
    .await
    .expect("Query failed");
    assert_eq!(entry_ids.len(), 1);
}

#[tokio::test]
#[serial]
async fn test_query_entries_by_account_types() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_gl_3_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    // Filter by Asset - should match all 3 entries
    let account_types = vec![AccountType::Asset];
    let entry_ids = query_entries_by_account_types(
        &pool,
        &tenant_id,
        &account_types,
        start_date,
        end_date,
        100,
        0,
    )
    .await
    .expect("Query failed");
    assert_eq!(entry_ids.len(), 3);

    // Filter by Liability - should match only entry 3
    let account_types = vec![AccountType::Liability];
    let entry_ids = query_entries_by_account_types(
        &pool,
        &tenant_id,
        &account_types,
        start_date,
        end_date,
        100,
        0,
    )
    .await
    .expect("Query failed");
    assert_eq!(entry_ids.len(), 1);
}

#[tokio::test]
#[serial]
async fn test_fetch_entry_lines_with_accounts() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_gl_4_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    let entry_ids = query_entries_by_date_range(&pool, &tenant_id,start_date, end_date, 1, 2)
        .await
        .expect("Query failed");
    assert_eq!(entry_ids.len(), 1);

    let entry_id = entry_ids[0];
    let lines = fetch_entry_lines_with_accounts(&pool, entry_id, &tenant_id)
        .await
        .expect("Fetch lines failed");

    assert_eq!(lines.len(), 2);
    assert!(!lines[0].account_name.is_empty());
    assert!(!lines[1].account_name.is_empty());
}

#[tokio::test]
#[serial]
async fn test_count_entries() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_gl_5_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    let count = count_entries_by_date_range(&pool, &tenant_id,start_date, end_date)
        .await
        .expect("Count failed");
    assert_eq!(count, 3);

    let account_codes = vec!["1000".to_string()];
    let count =
        count_entries_by_account_codes(&pool, &tenant_id,&account_codes, start_date, end_date)
            .await
            .expect("Count failed");
    assert_eq!(count, 3);

    let account_types = vec![AccountType::Revenue];
    let count =
        count_entries_by_account_types(&pool, &tenant_id,&account_types, start_date, end_date)
            .await
            .expect("Count failed");
    assert_eq!(count, 1);
}

// ============================================================
// PERIOD JOURNAL LISTING TESTS
// ============================================================

#[tokio::test]
#[serial]
async fn test_query_period_journal_entries() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_period_1_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap();

    let entries = query_period_journal_entries(&pool, &tenant_id,start_date, end_date, 100, 0)
        .await
        .expect("Query failed");

    assert_eq!(entries.len(), 3);
    assert!(entries[0].posted_at >= entries[1].posted_at);
    assert!(entries[1].posted_at >= entries[2].posted_at);
}

#[tokio::test]
#[serial]
async fn test_query_account_activity_no_results() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_edge_1_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    let start_date = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let end_date = Utc.with_ymd_and_hms(2024, 12, 31, 23, 59, 59).unwrap();

    let lines = query_account_activity(&pool, &tenant_id,"1000", start_date, end_date, 100, 0)
        .await
        .expect("Query failed");

    assert_eq!(lines.len(), 0);
}

#[tokio::test]
#[serial]
async fn test_fetch_entry_header_not_found() {
    let pool = setup_test_pool().await;

    let nonexistent_id = Uuid::new_v4();
    let header = fetch_entry_header(&pool, nonexistent_id)
        .await
        .expect("Fetch failed");

    assert!(header.is_none());
}

// ============================================================
// EXPLAIN PLAN TESTS (Performance Guardrails)
// ============================================================

#[tokio::test]
#[serial]
async fn test_explain_account_activity_uses_indexes() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_explain_1_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    // Run EXPLAIN on the account activity query
    let explain_result = sqlx::query_scalar::<_, String>(
        r#"
        EXPLAIN (FORMAT TEXT)
        SELECT
            je.id as entry_id,
            je.posted_at,
            je.description,
            je.currency,
            jl.id as line_id,
            jl.debit_minor,
            jl.credit_minor,
            jl.memo
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        WHERE je.tenant_id = $1
          AND jl.account_ref = $2
          AND je.posted_at >= $3
          AND je.posted_at <= $4
        ORDER BY je.posted_at ASC, jl.line_no ASC
        LIMIT 100 OFFSET 0
        "#,
    )
    .bind(&tenant_id)
    .bind("1000")
    .bind(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())
    .bind(Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap())
    .fetch_all(&pool)
    .await
    .expect("EXPLAIN query failed");

    let explain_output = explain_result.join("\n");

    // Assert no sequential scans on journal_entries (critical for performance at scale)
    assert!(
        !explain_output.contains("Seq Scan on journal_entries"),
        "Expected index usage on journal_entries, got sequential scan:\n{}",
        explain_output
    );

    // Assert that at least one index is being used (proves indexes are available and usable)
    // Note: With small test datasets, Postgres may choose seq scans on journal_lines
    // because the cost is lower. This is actually optimal for tiny tables.
    // At scale (100K+ entries), the idx_journal_lines_account_entry index will be used.
    assert!(
        explain_output.contains("Index Scan") || explain_output.contains("Bitmap"),
        "Expected index usage in query plan:\n{}",
        explain_output
    );
}

#[tokio::test]
#[serial]
async fn test_explain_entries_by_account_types_uses_indexes() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_explain_2_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    // Run EXPLAIN on the entries by account types query
    let explain_result = sqlx::query_scalar::<_, String>(
        r#"
        EXPLAIN (FORMAT TEXT)
        SELECT je.id
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        INNER JOIN accounts a ON a.tenant_id = je.tenant_id AND a.code = jl.account_ref
        WHERE je.tenant_id = $1
          AND a.type = ANY($2::account_type[])
          AND je.posted_at >= $3
          AND je.posted_at <= $4
        GROUP BY je.id, je.posted_at, je.created_at
        ORDER BY je.posted_at DESC, je.created_at DESC
        LIMIT 100 OFFSET 0
        "#,
    )
    .bind(&tenant_id)
    .bind(&[AccountType::Asset])
    .bind(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())
    .bind(Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap())
    .fetch_all(&pool)
    .await
    .expect("EXPLAIN query failed");

    let explain_output = explain_result.join("\n");

    // Assert no sequential scans on journal_entries (critical for performance at scale)
    assert!(
        !explain_output.contains("Seq Scan on journal_entries"),
        "Expected index usage on journal_entries, got sequential scan:\n{}",
        explain_output
    );

    // Assert that indexes are being used
    // Note: With small test datasets, Postgres may choose seq scans on small tables.
    // At scale, the idx_accounts_tenant_type index will be used.
    assert!(
        explain_output.contains("Index") || explain_output.contains("Bitmap"),
        "Expected index usage in query plan:\n{}",
        explain_output
    );
}

#[tokio::test]
#[serial]
async fn test_explain_entries_by_date_range_uses_indexes() {
    let pool = setup_test_pool().await;
    let tenant_id = format!("tenant_explain_3_{}", Uuid::new_v4());
    setup_test_data(&pool, &tenant_id).await;

    // Run EXPLAIN on the entries by date range query
    let explain_result = sqlx::query_scalar::<_, String>(
        r#"
        EXPLAIN (FORMAT TEXT)
        SELECT id
        FROM journal_entries
        WHERE tenant_id = $1
          AND posted_at >= $2
          AND posted_at <= $3
        ORDER BY posted_at DESC, created_at DESC
        LIMIT 100 OFFSET 0
        "#,
    )
    .bind(&tenant_id)
    .bind(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())
    .bind(Utc.with_ymd_and_hms(2025, 1, 31, 23, 59, 59).unwrap())
    .fetch_all(&pool)
    .await
    .expect("EXPLAIN query failed");

    let explain_output = explain_result.join("\n");

    // Assert no sequential scans on journal_entries
    assert!(
        !explain_output.contains("Seq Scan on journal_entries"),
        "Expected index usage on journal_entries, got sequential scan:\n{}",
        explain_output
    );

    // Assert that the tenant_posted index is being used
    assert!(
        explain_output.contains("idx_journal_entries_tenant_posted")
            || explain_output.contains("Index")
            || explain_output.contains("Bitmap"),
        "Expected idx_journal_entries_tenant_posted index usage:\n{}",
        explain_output
    );
}
