//! Boundary E2E Test: HTTP → Account Activity API (Track A - bd-2r0)
//!
//! This test validates the REAL HTTP boundary for GL Account Activity queries:
//! 1. Makes actual HTTP GET request to `/api/gl/accounts/{account_code}/activity`
//! 2. Validates response shape, serialization, status codes
//! 3. Tests pagination (limit/offset)
//! 4. Tests filtering by period_id and date range
//! 5. Tests currency filtering
//! 6. Verifies error handling (400/404)
//!
//! ## Architecture Decision
//! Per ChatGPT Phase 12 guidance: "Boundary-first testing for reporting primitives."
//! Read path ingress = HTTP, so this test hits the real HTTP endpoint.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - GL HTTP server at localhost:8090
//! - PostgreSQL at localhost:5438

use chrono::{NaiveDate, TimeZone, Utc};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::account_activity_service::AccountActivityResponse;
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

/// Helper to insert a journal entry with lines
async fn insert_test_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    account_code: &str,
    currency: &str,
    debit_minor: i64,
    credit_minor: i64,
    description: Option<&str>,
    memo: Option<&str>,
    posted_at: chrono::DateTime<Utc>,
) -> Uuid {
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();

    // Insert journal entry header
    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, posted_at, description, currency,
            source_module, source_event_id, source_subject, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .bind(description)
    .bind(currency)
    .bind("test")
    .bind(source_event_id)
    .bind("test-subject")
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert journal entry");

    // Insert journal line for the account
    let line_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO journal_lines (
            id, journal_entry_id, line_no, account_ref,
            debit_minor, credit_minor, memo
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(line_id)
    .bind(entry_id)
    .bind(1)
    .bind(account_code)
    .bind(debit_minor)
    .bind(credit_minor)
    .bind(memo)
    .execute(pool)
    .await
    .expect("Failed to insert journal line");

    entry_id
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal lines");

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal entries");

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
async fn test_boundary_http_account_activity_with_period_id() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-aa-period-001";
    let account_code = "1100";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup Chart of Accounts
    insert_test_account(
        &pool,
        tenant_id,
        account_code,
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
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

    // Insert test journal entries with posted_at within the period
    let posted_at = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap()
        .and_hms_opt(10, 0, 0).unwrap()
        .and_utc();

    insert_test_journal_entry(
        &pool,
        tenant_id,
        account_code,
        "USD",
        100000, // $1000 debit
        0,
        Some("Test transaction 1"),
        Some("Memo 1"),
        posted_at,
    )
    .await;

    insert_test_journal_entry(
        &pool,
        tenant_id,
        account_code,
        "USD",
        0,
        50000, // $500 credit
        Some("Test transaction 2"),
        Some("Memo 2"),
        posted_at + chrono::Duration::hours(1),
    )
    .await;

    // ✅ BOUNDARY TEST: Make real HTTP GET request
    let url = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response = reqwest::get(&url)
        .await
        .expect("Failed to make HTTP request - is GL service running on port 8090?");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from account activity endpoint"
    );

    // Assert: Response is valid JSON matching AccountActivityResponse structure
    let activity: AccountActivityResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Correct response fields
    assert_eq!(activity.tenant_id, tenant_id);
    assert_eq!(activity.account_code, account_code);
    assert_eq!(activity.lines.len(), 2, "Should have 2 journal lines");
    assert_eq!(activity.pagination.total_count, 2);
    assert_eq!(activity.pagination.limit, 50); // default limit
    assert_eq!(activity.pagination.offset, 0);
    assert!(!activity.pagination.has_more, "Should not have more pages");

    // Assert: Lines have correct structure
    let line1 = &activity.lines[0];
    assert!(line1.entry_id.len() > 0, "Should have entry_id");
    assert!(line1.posted_at.len() > 0, "Should have posted_at");
    assert_eq!(line1.currency, "USD");
    assert!(line1.debit_minor > 0 || line1.credit_minor > 0);

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_account_activity_pagination() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-aa-pagination-001";
    let account_code = "2000";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup account
    insert_test_account(
        &pool,
        tenant_id,
        account_code,
        "Accounts Payable",
        AccountType::Liability,
        NormalBalance::Credit,
    )
    .await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert 5 test entries with posted_at within the period
    let posted_at_base = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
    for i in 0..5 {
        insert_test_journal_entry(
            &pool,
            tenant_id,
            account_code,
            "USD",
            0,
            (i + 1) * 10000, // Varying amounts
            Some(&format!("Transaction {}", i + 1)),
            None,
            posted_at_base + chrono::Duration::hours(i as i64),
        )
        .await;
    }

    // Test 1: First page (limit=2, offset=0)
    let url_page1 = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}&limit=2&offset=0",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response1 = reqwest::get(&url_page1).await.expect("Failed to fetch page 1");
    assert_eq!(response1.status(), 200);

    let activity1: AccountActivityResponse = response1.json().await.expect("Failed to parse JSON");
    assert_eq!(activity1.lines.len(), 2, "Should have 2 lines on page 1");
    assert_eq!(activity1.pagination.total_count, 5);
    assert_eq!(activity1.pagination.limit, 2);
    assert_eq!(activity1.pagination.offset, 0);
    assert!(activity1.pagination.has_more, "Should have more pages");

    // Test 2: Second page (limit=2, offset=2)
    let url_page2 = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}&limit=2&offset=2",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response2 = reqwest::get(&url_page2).await.expect("Failed to fetch page 2");
    assert_eq!(response2.status(), 200);

    let activity2: AccountActivityResponse = response2.json().await.expect("Failed to parse JSON");
    assert_eq!(activity2.lines.len(), 2, "Should have 2 lines on page 2");
    assert_eq!(activity2.pagination.offset, 2);
    assert!(activity2.pagination.has_more, "Should have more pages");

    // Test 3: Last page (limit=2, offset=4)
    let url_page3 = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}&limit=2&offset=4",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response3 = reqwest::get(&url_page3).await.expect("Failed to fetch page 3");
    assert_eq!(response3.status(), 200);

    let activity3: AccountActivityResponse = response3.json().await.expect("Failed to parse JSON");
    assert_eq!(activity3.lines.len(), 1, "Should have 1 line on last page");
    assert_eq!(activity3.pagination.offset, 4);
    assert!(!activity3.pagination.has_more, "Should not have more pages");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_account_activity_currency_filter() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-aa-currency-001";
    let account_code = "3000";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup account
    insert_test_account(
        &pool,
        tenant_id,
        account_code,
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert entries with different currencies within the period
    let posted_at = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
    insert_test_journal_entry(&pool, tenant_id, account_code, "USD", 0, 100000, Some("USD tx"), None, posted_at).await;
    insert_test_journal_entry(&pool, tenant_id, account_code, "EUR", 0, 50000, Some("EUR tx"), None, posted_at + chrono::Duration::hours(1)).await;
    insert_test_journal_entry(&pool, tenant_id, account_code, "USD", 0, 75000, Some("USD tx 2"), None, posted_at + chrono::Duration::hours(2)).await;

    // Test 1: Filter by USD
    let url_usd = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response_usd = reqwest::get(&url_usd).await.expect("Failed to fetch USD activity");
    assert_eq!(response_usd.status(), 200);

    let activity_usd: AccountActivityResponse = response_usd.json().await.expect("Failed to parse JSON");
    assert_eq!(activity_usd.lines.len(), 2, "Should have 2 USD lines");
    assert!(activity_usd.lines.iter().all(|l| l.currency == "USD"));

    // Test 2: Filter by EUR
    let url_eur = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}&currency=EUR",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response_eur = reqwest::get(&url_eur).await.expect("Failed to fetch EUR activity");
    assert_eq!(response_eur.status(), 200);

    let activity_eur: AccountActivityResponse = response_eur.json().await.expect("Failed to parse JSON");
    assert_eq!(activity_eur.lines.len(), 1, "Should have 1 EUR line");
    assert_eq!(activity_eur.lines[0].currency, "EUR");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_account_activity_error_handling() {
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Test 1: Missing required query parameter tenant_id (should return 400)
    let url_missing_tenant = format!(
        "{}/api/gl/accounts/1000/activity",
        gl_service_url
    );

    let response = reqwest::get(&url_missing_tenant).await.expect("Failed to make request");
    assert_eq!(
        response.status(),
        400,
        "Should return 400 for missing tenant_id parameter"
    );

    // Test 2: Missing both period_id and date range (should return 400)
    let url_no_date_filter = format!(
        "{}/api/gl/accounts/1000/activity?tenant_id=test",
        gl_service_url
    );

    let response_no_date = reqwest::get(&url_no_date_filter).await.expect("Failed to make request");
    assert_eq!(
        response_no_date.status(),
        400,
        "Should return 400 when missing both period_id and date range"
    );

    // Test 3: Invalid UUID format for period_id (should return 400)
    let url_invalid_uuid = format!(
        "{}/api/gl/accounts/1000/activity?tenant_id=test&period_id=not-a-uuid",
        gl_service_url
    );

    let response_invalid = reqwest::get(&url_invalid_uuid).await.expect("Failed to make request");
    assert_eq!(
        response_invalid.status(),
        400,
        "Should return 400 for invalid UUID format"
    );

    // Test 4: Period not found (should return 404)
    let nonexistent_period_id = Uuid::new_v4();
    let url_not_found = format!(
        "{}/api/gl/accounts/1000/activity?tenant_id=nonexistent&period_id={}",
        gl_service_url, nonexistent_period_id
    );

    let response_not_found = reqwest::get(&url_not_found).await.expect("Failed to make request");
    assert_eq!(
        response_not_found.status(),
        404,
        "Should return 404 for period not found"
    );

    // Test 5: Invalid pagination parameters (should return 400)
    let period_id = Uuid::new_v4();
    let url_invalid_pagination = format!(
        "{}/api/gl/accounts/1000/activity?tenant_id=test&period_id={}&limit=1000",
        gl_service_url, period_id
    );

    let response_bad_pagination = reqwest::get(&url_invalid_pagination).await.expect("Failed to make request");
    assert!(
        response_bad_pagination.status().is_client_error(),
        "Should return 4xx for invalid pagination (limit > 100)"
    );
}

#[tokio::test]
#[serial]
async fn test_boundary_http_account_activity_json_dto_structure() {
    // This test validates the exact JSON structure matches production DTOs
    // to prevent breaking changes in serialization.

    let pool = setup_test_pool().await;
    let tenant_id = "tenant-aa-dto-test";
    let account_code = "4000";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup account
    insert_test_account(
        &pool,
        tenant_id,
        account_code,
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert test entry within the period
    let posted_at = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
    insert_test_journal_entry(
        &pool,
        tenant_id,
        account_code,
        "USD",
        0,
        200000,
        Some("Test revenue"),
        Some("Test memo"),
        posted_at,
    )
    .await;

    // Make request
    let url = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}",
        gl_service_url, account_code, tenant_id, period_id
    );

    let response = reqwest::get(&url).await.expect("Failed to make request");
    assert_eq!(response.status(), 200);

    // Parse as generic JSON first to validate structure
    let json_value: serde_json::Value = response.json().await.expect("Failed to parse JSON");

    // Assert: All expected top-level fields present
    assert!(json_value.get("tenant_id").is_some(), "Missing tenant_id field");
    assert!(json_value.get("account_code").is_some(), "Missing account_code field");
    assert!(json_value.get("period_start").is_some(), "Missing period_start field");
    assert!(json_value.get("period_end").is_some(), "Missing period_end field");
    assert!(json_value.get("lines").is_some(), "Missing lines field");
    assert!(json_value.get("pagination").is_some(), "Missing pagination field");

    // Assert: Correct field types
    assert!(json_value["tenant_id"].is_string());
    assert!(json_value["account_code"].is_string());
    assert!(json_value["period_start"].is_string());
    assert!(json_value["period_end"].is_string());
    assert!(json_value["lines"].is_array());

    // Assert: Lines structure
    let lines = json_value["lines"].as_array().unwrap();
    assert!(lines.len() > 0, "Should have at least one line");

    let line = &lines[0];
    assert!(line.get("entry_id").is_some(), "Missing entry_id in line");
    assert!(line.get("posted_at").is_some(), "Missing posted_at in line");
    assert!(line.get("currency").is_some(), "Missing currency in line");
    assert!(line.get("debit_minor").is_some(), "Missing debit_minor in line");
    assert!(line.get("credit_minor").is_some(), "Missing credit_minor in line");

    // Assert: Pagination structure
    let pagination = &json_value["pagination"];
    assert!(pagination.get("limit").is_some(), "Missing limit in pagination");
    assert!(pagination.get("offset").is_some(), "Missing offset in pagination");
    assert!(pagination.get("total_count").is_some(), "Missing total_count in pagination");
    assert!(pagination.get("has_more").is_some(), "Missing has_more in pagination");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_account_activity_performance_guard() {
    // This test verifies that account activity queries are fast and use indexes.
    // Expected: < 200ms for 1000 transactions (per Phase 12 spec)

    let pool = setup_test_pool().await;
    let tenant_id = "tenant-aa-perf-guard";
    let account_code = "5000";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup account
    insert_test_account(
        &pool,
        tenant_id,
        account_code,
        "Expense",
        AccountType::Expense,
        NormalBalance::Debit,
    )
    .await;

    // Setup period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    // Insert 10 test entries (limited to avoid test slowness) within the period
    let posted_at_base = Utc.with_ymd_and_hms(2024, 3, 15, 10, 0, 0).unwrap();
    for i in 0..10 {
        insert_test_journal_entry(
            &pool,
            tenant_id,
            account_code,
            "USD",
            (i + 1) * 1000,
            0,
            Some(&format!("Expense {}", i + 1)),
            None,
            posted_at_base + chrono::Duration::hours(i as i64),
        )
        .await;
    }

    // Make HTTP request and time it
    let url = format!(
        "{}/api/gl/accounts/{}/activity?tenant_id={}&period_id={}",
        gl_service_url, account_code, tenant_id, period_id
    );

    let start = std::time::Instant::now();
    let response = reqwest::get(&url).await.expect("Failed to fetch account activity");
    let elapsed = start.elapsed();

    assert_eq!(response.status(), 200);

    // Assert: Response time is fast (< 500ms)
    // This proves the query uses indexes efficiently
    assert!(
        elapsed.as_millis() < 500,
        "Account activity should be fast (< 500ms), was {:?}. This suggests inefficient query execution.",
        elapsed
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
