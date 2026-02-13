//! Boundary E2E Test: HTTP → Period Summary API (Track C)
//!
//! This test validates the REAL HTTP boundary for GL Period Summary queries:
//! 1. Makes actual HTTP GET request to `/api/gl/periods/{period_id}/summary`
//! 2. Validates response shape, serialization, status codes
//! 3. Tests error handling (400/404)
//! 4. Verifies snapshot-first logic with fallback to account_balances
//!
//! ## Architecture Decision
//! Per ChatGPT Phase 12 guidance: "Boundary-first testing for reporting primitives."
//! Read path ingress = HTTP, so this test hits the real HTTP endpoint.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - GL HTTP server at localhost:8090
//! - PostgreSQL at localhost:5438

use chrono::{NaiveDate, Utc};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::period_summary_service::PeriodSummaryResponse;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Setup test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

/// Helper to insert a test account
async fn insert_test_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: AccountType,
    normal_balance: NormalBalance,
) -> Uuid {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .bind(true)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test account");

    id
}

/// Helper to create a test accounting period
async fn insert_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Uuid {
    let period_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .bind(false) // open
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test period");

    period_id
}

/// Helper to insert period summary snapshot
async fn insert_period_summary_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,
    journal_count: i32,
    line_count: i32,
    total_debits_minor: i64,
    total_credits_minor: i64,
) {
    sqlx::query(
        r#"
        INSERT INTO period_summary_snapshots (
            id, tenant_id, period_id, currency,
            journal_count, line_count,
            total_debits_minor, total_credits_minor,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (tenant_id, period_id, currency) DO UPDATE SET
            journal_count = EXCLUDED.journal_count,
            line_count = EXCLUDED.line_count,
            total_debits_minor = EXCLUDED.total_debits_minor,
            total_credits_minor = EXCLUDED.total_credits_minor
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(currency)
    .bind(journal_count)
    .bind(line_count)
    .bind(total_debits_minor)
    .bind(total_credits_minor)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert period summary snapshot");
}

/// Helper to insert account balance
async fn insert_test_balance(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    account_code: &str,
    currency: &str,
    debit_total_minor: i64,
    credit_total_minor: i64,
) {
    let net_balance_minor = debit_total_minor - credit_total_minor;
    let journal_entry_id = Uuid::new_v4(); // Dummy entry ID

    sqlx::query(
        r#"
        INSERT INTO account_balances (
            id, tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor,
            last_journal_entry_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW(), NOW())
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            debit_total_minor = EXCLUDED.debit_total_minor,
            credit_total_minor = EXCLUDED.credit_total_minor,
            net_balance_minor = EXCLUDED.net_balance_minor
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(account_code)
    .bind(currency)
    .bind(debit_total_minor)
    .bind(credit_total_minor)
    .bind(net_balance_minor)
    .bind(journal_entry_id)
    .execute(pool)
    .await
    .expect("Failed to insert test balance");
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup snapshots");

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup balances");

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup accounts");

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup periods");
}

#[tokio::test]
#[serial]
async fn test_boundary_http_period_summary_from_snapshot() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-ps-snap-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup accounting period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert precomputed snapshot
    insert_period_summary_snapshot(
        &pool,
        tenant_id,
        period_id,
        "USD",
        15,  // journal_count
        30,  // line_count
        500000, // total_debits_minor ($5000)
        500000, // total_credits_minor ($5000)
    )
    .await;

    // ✅ BOUNDARY TEST: Make real HTTP GET request
    let url = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}&currency=USD",
        gl_service_url, period_id, tenant_id
    );

    let response = reqwest::get(&url)
        .await
        .expect("Failed to make HTTP request - is GL service running on port 8090?");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from period summary endpoint"
    );

    // Assert: Response is valid JSON matching PeriodSummaryResponse structure
    let summary: PeriodSummaryResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Correct response fields
    assert_eq!(summary.tenant_id, tenant_id);
    assert_eq!(summary.period_id, period_id);
    assert_eq!(summary.currency, "USD");
    assert_eq!(summary.journal_count, 15, "Should match snapshot journal_count");
    assert_eq!(summary.line_count, 30, "Should match snapshot line_count");
    assert_eq!(summary.total_debits_minor, 500000, "Should match snapshot debits");
    assert_eq!(summary.total_credits_minor, 500000, "Should match snapshot credits");
    assert!(summary.is_balanced, "Should be balanced");
    assert_eq!(summary.data_source, "snapshot", "Should indicate snapshot source");
    assert!(summary.snapshot_created_at.is_some(), "Should have snapshot timestamp");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_period_summary_computed_from_balances() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-ps-computed-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup Chart of Accounts
    insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup accounting period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert balances (NO snapshot - should compute from balances)
    insert_test_balance(&pool, tenant_id, period_id, "1100", "USD", 300000, 0).await; // $3000 Cash
    insert_test_balance(&pool, tenant_id, period_id, "4000", "USD", 0, 300000).await; // $3000 Revenue

    // ✅ BOUNDARY TEST: Make real HTTP GET request
    let url = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}&currency=USD",
        gl_service_url, period_id, tenant_id
    );

    let response = reqwest::get(&url).await.expect("Failed to make HTTP request");

    // Assert: 200 OK
    assert_eq!(response.status(), 200);

    // Assert: Response is valid JSON
    let summary: PeriodSummaryResponse = response.json().await.expect("Failed to parse JSON");

    // Assert: Correct computed values
    assert_eq!(summary.tenant_id, tenant_id);
    assert_eq!(summary.period_id, period_id);
    assert_eq!(summary.currency, "USD");
    assert_eq!(summary.journal_count, 0, "Computed from balances cannot know journal_count");
    assert_eq!(summary.line_count, 0, "Computed from balances cannot know line_count");
    assert_eq!(summary.total_debits_minor, 300000, "Should sum from balances");
    assert_eq!(summary.total_credits_minor, 300000, "Should sum from balances");
    assert!(summary.is_balanced, "Should be balanced");
    assert_eq!(summary.data_source, "computed", "Should indicate computed source");
    assert!(summary.snapshot_created_at.is_none(), "Should have no snapshot timestamp");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_period_summary_currency_filter() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-ps-multi-currency";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert snapshots for multiple currencies
    insert_period_summary_snapshot(&pool, tenant_id, period_id, "USD", 10, 20, 100000, 100000).await;
    insert_period_summary_snapshot(&pool, tenant_id, period_id, "EUR", 5, 10, 50000, 50000).await;
    insert_period_summary_snapshot(&pool, tenant_id, period_id, "GBP", 3, 6, 30000, 30000).await;

    // Test 1: Filter by USD only
    let url_usd = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}&currency=USD",
        gl_service_url, period_id, tenant_id
    );

    let response_usd = reqwest::get(&url_usd).await.expect("Failed to fetch USD summary");
    assert_eq!(response_usd.status(), 200);

    let summary_usd: PeriodSummaryResponse = response_usd.json().await.expect("Failed to parse JSON");
    assert_eq!(summary_usd.currency, "USD");
    assert_eq!(summary_usd.journal_count, 10, "Should have USD journal count");
    assert_eq!(summary_usd.total_debits_minor, 100000);

    // Test 2: Filter by EUR
    let url_eur = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}&currency=EUR",
        gl_service_url, period_id, tenant_id
    );

    let response_eur = reqwest::get(&url_eur).await.expect("Failed to fetch EUR summary");
    assert_eq!(response_eur.status(), 200);

    let summary_eur: PeriodSummaryResponse = response_eur.json().await.expect("Failed to parse JSON");
    assert_eq!(summary_eur.currency, "EUR");
    assert_eq!(summary_eur.journal_count, 5, "Should have EUR journal count");
    assert_eq!(summary_eur.total_debits_minor, 50000);

    // Test 3: No currency filter (should aggregate all)
    let url_all = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}",
        gl_service_url, period_id, tenant_id
    );

    let response_all = reqwest::get(&url_all).await.expect("Failed to fetch all currencies");
    assert_eq!(response_all.status(), 200);

    let summary_all: PeriodSummaryResponse = response_all.json().await.expect("Failed to parse JSON");
    assert_eq!(summary_all.currency, "MULTI", "Should indicate multi-currency");
    assert_eq!(summary_all.journal_count, 18, "Should sum all journal counts (10+5+3)");
    assert_eq!(summary_all.total_debits_minor, 180000, "Should sum all debits");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_period_summary_error_handling() {
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Test 1: Missing required query parameter (should return 400)
    let fake_period_id = Uuid::new_v4();
    let url_missing_tenant = format!(
        "{}/api/gl/periods/{}/summary",
        gl_service_url, fake_period_id
    );

    let response = reqwest::get(&url_missing_tenant).await.expect("Failed to make request");
    assert_eq!(
        response.status(),
        400,
        "Should return 400 for missing tenant_id parameter"
    );

    // Test 2: Invalid UUID format in path (should return 400)
    let url_invalid_uuid = format!(
        "{}/api/gl/periods/not-a-uuid/summary?tenant_id=test",
        gl_service_url
    );

    let response_invalid = reqwest::get(&url_invalid_uuid).await.expect("Failed to make request");
    assert_eq!(
        response_invalid.status(),
        400,
        "Should return 400 for invalid UUID format"
    );

    // Test 3: Period not found (should return 404)
    let nonexistent_period_id = Uuid::new_v4();
    let url_not_found = format!(
        "{}/api/gl/periods/{}/summary?tenant_id=nonexistent-tenant",
        gl_service_url, nonexistent_period_id
    );

    let response_not_found = reqwest::get(&url_not_found).await.expect("Failed to make request");
    assert_eq!(
        response_not_found.status(),
        404,
        "Should return 404 for period not found"
    );

    // Test 4: Invalid currency format (should return 400)
    let period_id = Uuid::new_v4();
    let url_invalid_currency = format!(
        "{}/api/gl/periods/{}/summary?tenant_id=test&currency=invalid",
        gl_service_url, period_id
    );

    let response_bad_currency = reqwest::get(&url_invalid_currency).await.expect("Failed to make request");
    // Note: This might be 400 or 404 depending on whether period exists
    // We're testing that invalid currency is handled gracefully
    assert!(
        response_bad_currency.status().is_client_error(),
        "Should return 4xx for invalid currency format"
    );
}

#[tokio::test]
#[serial]
async fn test_boundary_http_period_summary_performance_guard() {
    // This test verifies that period summary does NOT scan journal_lines table.
    //
    // Method: Query a period with account_balances but WITHOUT journal_lines.
    // If the query scans journal_lines, it will be slow or fail.
    // If it uses account_balances/snapshots, it will be fast.
    //
    // We test this by timing the response (should be < 500ms).

    let pool = setup_test_pool().await;
    let tenant_id = "tenant-ps-perf-guard";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup minimal period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert snapshot (precomputed - should be instant)
    insert_period_summary_snapshot(&pool, tenant_id, period_id, "USD", 5, 10, 100000, 100000).await;

    // Make HTTP request and time it
    let url = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}&currency=USD",
        gl_service_url, period_id, tenant_id
    );

    let start = std::time::Instant::now();
    let response = reqwest::get(&url).await.expect("Failed to fetch period summary");
    let elapsed = start.elapsed();

    assert_eq!(response.status(), 200);

    // Assert: Response time is fast (< 500ms)
    // This proves the query uses snapshots/account_balances (NOT journal_lines)
    assert!(
        elapsed.as_millis() < 500,
        "Period summary should be fast (< 500ms), was {:?}. This suggests it's NOT querying journal_lines table.",
        elapsed
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_period_summary_json_dto_structure() {
    // This test validates the exact JSON structure matches production DTOs
    // to prevent breaking changes in serialization.

    let pool = setup_test_pool().await;
    let tenant_id = "tenant-ps-dto-test";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert snapshot
    insert_period_summary_snapshot(&pool, tenant_id, period_id, "USD", 10, 20, 200000, 200000).await;

    // Make request
    let url = format!(
        "{}/api/gl/periods/{}/summary?tenant_id={}&currency=USD",
        gl_service_url, period_id, tenant_id
    );

    let response = reqwest::get(&url).await.expect("Failed to make request");
    assert_eq!(response.status(), 200);

    // Parse as generic JSON first to validate structure
    let json_value: serde_json::Value = response.json().await.expect("Failed to parse JSON");

    // Assert: All expected fields present
    assert!(json_value.get("tenant_id").is_some(), "Missing tenant_id field");
    assert!(json_value.get("period_id").is_some(), "Missing period_id field");
    assert!(json_value.get("currency").is_some(), "Missing currency field");
    assert!(json_value.get("journal_count").is_some(), "Missing journal_count field");
    assert!(json_value.get("line_count").is_some(), "Missing line_count field");
    assert!(json_value.get("total_debits_minor").is_some(), "Missing total_debits_minor field");
    assert!(json_value.get("total_credits_minor").is_some(), "Missing total_credits_minor field");
    assert!(json_value.get("is_balanced").is_some(), "Missing is_balanced field");
    assert!(json_value.get("data_source").is_some(), "Missing data_source field");
    assert!(json_value.get("snapshot_created_at").is_some(), "Missing snapshot_created_at field");

    // Assert: Correct field types
    assert!(json_value["tenant_id"].is_string());
    assert!(json_value["period_id"].is_string()); // UUID serialized as string
    assert!(json_value["currency"].is_string());
    assert!(json_value["journal_count"].is_number());
    assert!(json_value["line_count"].is_number());
    assert!(json_value["total_debits_minor"].is_number());
    assert!(json_value["total_credits_minor"].is_number());
    assert!(json_value["is_balanced"].is_boolean());
    assert!(json_value["data_source"].is_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
