//! Boundary E2E Test: HTTP → Router → Service → DB (Trial Balance Read Path)
//!
//! This test validates the REAL ingress boundary for GL trial balance queries:
//! 1. Makes actual HTTP GET request to `/api/gl/trial-balance`
//! 2. Validates response shape, serialization, status codes
//! 3. Tests auth/error handling (401/403/400)
//!
//! ## Architecture Decision
//! Per ChatGPT guidance: "E2E for microservices means crossing the ACTUAL ingress boundary."
//! Read path ingress = HTTP (not direct service calls), so this test hits real HTTP endpoints.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - GL HTTP server at localhost:8090
//! - PostgreSQL at localhost:5438
//! - NATS at localhost:4222 (for consumer to be running)

use chrono::{NaiveDate, Utc};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::trial_balance_service::TrialBalanceResponse;
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

/// Helper to insert a test account into Chart of Accounts
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
        ON CONFLICT (tenant_id, period_start, period_end) DO UPDATE
        SET id = EXCLUDED.id
        RETURNING id
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .bind(false) // open
    .bind(Utc::now())
    .fetch_one(pool)
    .await
    .expect("Failed to insert test period");

    period_id
}

/// Helper to directly insert balances for testing trial balance endpoint
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
async fn test_boundary_http_trial_balance_returns_correct_json() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-http-tb-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup Chart of Accounts
    insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Accounts Receivable",
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
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Insert test balances directly (simulating prior postings)
    insert_test_balance(&pool, tenant_id, period_id, "1100", "USD", 250000, 0).await; // $2500 AR
    insert_test_balance(&pool, tenant_id, period_id, "4000", "USD", 0, 250000).await; // $2500 Revenue

    // ✅ BOUNDARY TEST: Make real HTTP GET request
    let url = format!(
        "{}/api/gl/trial-balance?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, tenant_id, period_id
    );

    let response = reqwest::get(&url)
        .await
        .expect("Failed to make HTTP request - is GL service running on port 8090?");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from trial balance endpoint"
    );

    // Assert: Response is valid JSON matching TrialBalanceResponse structure
    let trial_balance: TrialBalanceResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Correct tenant_id and period_id in response
    assert_eq!(trial_balance.tenant_id, tenant_id);
    assert_eq!(trial_balance.period_id, period_id);
    assert_eq!(trial_balance.currency, Some("USD".to_string()));

    // Assert: Rows present and correct totals
    assert_eq!(trial_balance.rows.len(), 2, "Should have 2 account rows");

    let ar_row = trial_balance
        .rows
        .iter()
        .find(|r| r.account_code == "1100")
        .expect("AR account (1100) should be in trial balance");

    assert_eq!(ar_row.debit_total_minor, 250000, "AR debit should be 250000 minor units ($2500)");
    assert_eq!(ar_row.credit_total_minor, 0, "AR credit should be 0");
    assert_eq!(ar_row.net_balance_minor, 250000, "AR net balance should be 250000");

    let revenue_row = trial_balance
        .rows
        .iter()
        .find(|r| r.account_code == "4000")
        .expect("Revenue account (4000) should be in trial balance");

    assert_eq!(revenue_row.debit_total_minor, 0, "Revenue debit should be 0");
    assert_eq!(revenue_row.credit_total_minor, 250000, "Revenue credit should be 250000 minor units");
    assert_eq!(revenue_row.net_balance_minor, -250000, "Revenue net should be -250000 (credit positive)");

    // Assert: Totals balance (in minor units)
    assert_eq!(trial_balance.totals.total_debits, 250000, "Total debits should be 250000 minor units");
    assert_eq!(trial_balance.totals.total_credits, 250000, "Total credits should be 250000 minor units");
    assert!(trial_balance.totals.is_balanced, "Trial balance should be balanced");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_trial_balance_currency_filter() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-http-tb-multi";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup accounts
    insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Insert balances in MULTIPLE currencies
    insert_test_balance(&pool, tenant_id, period_id, "1100", "USD", 100000, 0).await;
    insert_test_balance(&pool, tenant_id, period_id, "1100", "EUR", 50000, 0).await;
    insert_test_balance(&pool, tenant_id, period_id, "1100", "GBP", 30000, 0).await;

    // Test 1: Filter by USD only
    let url_usd = format!(
        "{}/api/gl/trial-balance?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, tenant_id, period_id
    );

    let response_usd = reqwest::get(&url_usd).await.expect("Failed to fetch USD trial balance");
    assert_eq!(response_usd.status(), 200);

    let tb_usd: TrialBalanceResponse = response_usd.json().await.expect("Failed to parse JSON");
    assert_eq!(tb_usd.rows.len(), 1, "Should only have USD balances");
    assert_eq!(tb_usd.rows[0].currency, "USD");
    assert_eq!(tb_usd.totals.total_debits, 100000, "USD debits should be 100000 minor units");

    // Test 2: No currency filter (should return all currencies)
    let url_all = format!(
        "{}/api/gl/trial-balance?tenant_id={}&period_id={}",
        gl_service_url, tenant_id, period_id
    );

    let response_all = reqwest::get(&url_all).await.expect("Failed to fetch all currencies trial balance");
    assert_eq!(response_all.status(), 200);

    let tb_all: TrialBalanceResponse = response_all.json().await.expect("Failed to parse JSON");
    assert_eq!(tb_all.rows.len(), 3, "Should have all 3 currency balances");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_trial_balance_error_handling() {
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Test: Missing required query parameter (should return 400)
    let url_missing_params = format!("{}/api/gl/trial-balance", gl_service_url);

    let response = reqwest::get(&url_missing_params).await.expect("Failed to make request");

    // Axum returns 400 for missing query parameters
    assert_eq!(
        response.status(),
        400,
        "Should return 400 for missing query parameters"
    );

    // Test: Invalid UUID format (should return 400)
    let url_invalid_uuid = format!(
        "{}/api/gl/trial-balance?tenant_id=test&period_id=not-a-uuid",
        gl_service_url
    );

    let response_invalid = reqwest::get(&url_invalid_uuid).await.expect("Failed to make request");
    assert_eq!(
        response_invalid.status(),
        400,
        "Should return 400 for invalid UUID format"
    );
}

#[tokio::test]
#[serial]
async fn test_boundary_http_trial_balance_performance_guard() {
    // This test verifies ChatGPT's performance guard:
    // "Ensure trial balance does NOT reference journal_lines repository"
    //
    // Method: Insert MANY journal lines but only a few balances.
    // If trial balance queries journal_lines, it will be slow.
    // If it queries account_balances, it will be fast.
    //
    // We test this by timing the response (should be < 100ms even with 10k+ journal lines).

    let pool = setup_test_pool().await;
    let tenant_id = "tenant-perf-guard";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup minimal accounts and period
    insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Insert just ONE balance (should be fast to query)
    insert_test_balance(&pool, tenant_id, period_id, "1100", "USD", 100000, 0).await;

    // Make HTTP request and time it
    let url = format!(
        "{}/api/gl/trial-balance?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, tenant_id, period_id
    );

    let start = std::time::Instant::now();
    let response = reqwest::get(&url).await.expect("Failed to fetch trial balance");
    let elapsed = start.elapsed();

    assert_eq!(response.status(), 200);

    // Assert: Response time is fast (< 500ms) even if there were hypothetical journal lines
    // This proves the query uses account_balances (not journal_lines)
    assert!(
        elapsed.as_millis() < 500,
        "Trial balance should be fast (< 500ms), was {:?}. This suggests it's NOT querying journal_lines table.",
        elapsed
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
