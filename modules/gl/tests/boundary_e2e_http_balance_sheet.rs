//! Boundary E2E Test: HTTP → Router → Service → DB (Balance Sheet Read Path)
//!
//! This test validates the REAL ingress boundary for GL balance sheet queries:
//! 1. Makes actual HTTP GET request to `/api/gl/balance-sheet`
//! 2. Validates response shape, serialization, status codes
//! 3. Tests accounting equation (Assets = Liabilities + Equity)
//! 4. Tests error handling (400 for missing params)
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
use gl_rs::services::balance_sheet_service::BalanceSheetResponse;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ── JWT test helper ──────────────────────────────────────────────────

/// JWT claims matching the platform's RawAccessClaims structure.
#[derive(Serialize)]
struct TestJwtClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

/// Sign a test JWT for the given tenant_id using the dev private key.
fn sign_test_jwt(tenant_id: &str) -> String {
    dotenvy::dotenv().ok();
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set (loaded from .env)");
    let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
        .expect("Invalid JWT_PRIVATE_KEY_PEM");

    let now = Utc::now();
    let claims = TestJwtClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["operator".into()],
        perms: vec!["gl.read".into()],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };

    let header = Header::new(Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &encoding_key)
        .expect("Failed to sign test JWT")
}

/// Build a reqwest Client that sends the Bearer token on every request.
fn authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token).parse().unwrap(),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

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

/// Helper to directly insert balances for testing balance sheet endpoint
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
#[ignore]
async fn test_boundary_http_balance_sheet_returns_correct_json() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_uuid = Uuid::new_v4();
    let tenant_id = tenant_uuid.to_string();
    let tenant_id = tenant_id.as_str();
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup Chart of Accounts (Assets = Liabilities + Equity)
    insert_test_account(
        &pool,
        tenant_id,
        "1000",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "2000",
        "Accounts Payable",
        AccountType::Liability,
        NormalBalance::Credit,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "3000",
        "Owner's Equity",
        AccountType::Equity,
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

    // Insert balanced test balances (Assets = $1000, Liabilities = $400, Equity = $600)
    insert_test_balance(&pool, tenant_id, period_id, "1000", "USD", 100000, 0).await; // $1000 asset
    insert_test_balance(&pool, tenant_id, period_id, "2000", "USD", 0, 40000).await; // $400 liability
    insert_test_balance(&pool, tenant_id, period_id, "3000", "USD", 0, 60000).await; // $600 equity

    // ✅ BOUNDARY TEST: Make real HTTP GET request (with JWT auth)
    let token = sign_test_jwt(tenant_id);
    let client = authed_client(&token);

    let url = format!(
        "{}/api/gl/balance-sheet?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, tenant_id, period_id
    );

    let response = client
        .get(&url)
        .send()
        .await
        .expect("Failed to make HTTP request - is GL service running on port 8090?");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from balance sheet endpoint"
    );

    // Assert: Response is valid JSON matching BalanceSheetResponse structure
    let balance_sheet: BalanceSheetResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Correct tenant_id and period_id in response
    assert_eq!(balance_sheet.tenant_id, tenant_id);
    assert_eq!(balance_sheet.period_id, period_id);
    assert_eq!(balance_sheet.currency, "USD");

    // Assert: Rows present and correct totals
    assert_eq!(
        balance_sheet.rows.len(),
        3,
        "Should have 3 account rows (assets + liabilities + equity)"
    );

    // Assert: Accounting equation (Assets = Liabilities + Equity)
    assert_eq!(
        balance_sheet.totals.total_assets, 100000,
        "Assets should be $1000 (100000 minor units)"
    );
    assert_eq!(
        balance_sheet.totals.total_liabilities, 40000,
        "Liabilities should be $400 (40000 minor units)"
    );
    assert_eq!(
        balance_sheet.totals.total_equity, 60000,
        "Equity should be $600 (60000 minor units)"
    );
    assert!(
        balance_sheet.totals.is_balanced,
        "Balance sheet should satisfy accounting equation (A = L + E)"
    );

    // Verify equation: 100000 = 40000 + 60000
    assert_eq!(
        balance_sheet.totals.total_assets,
        balance_sheet.totals.total_liabilities + balance_sheet.totals.total_equity,
        "Assets must equal Liabilities + Equity"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn test_boundary_http_balance_sheet_error_handling() {
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    let token = sign_test_jwt("tenant-error-test");
    let client = authed_client(&token);

    // Test: Missing required query parameter (should return 400)
    let url_missing_params = format!("{}/api/gl/balance-sheet", gl_service_url);

    let response = client
        .get(&url_missing_params)
        .send()
        .await
        .expect("Failed to make request");

    // Axum returns 400 for missing query parameters
    assert_eq!(
        response.status(),
        400,
        "Should return 400 for missing query parameters"
    );

    // Test: Invalid UUID format (should return 400)
    let url_invalid_uuid = format!(
        "{}/api/gl/balance-sheet?tenant_id=test&period_id=not-a-uuid&currency=USD",
        gl_service_url
    );

    let response_invalid = client
        .get(&url_invalid_uuid)
        .send()
        .await
        .expect("Failed to make request");
    assert_eq!(
        response_invalid.status(),
        400,
        "Should return 400 for invalid UUID format"
    );
}

#[tokio::test]
#[serial]
#[ignore]
async fn test_boundary_http_balance_sheet_schema_validation() {
    // This test verifies the JSON schema matches expectations (deterministic serialization)
    let pool = setup_test_pool().await;
    let tenant_uuid = Uuid::new_v4();
    let tenant_id = tenant_uuid.to_string();
    let tenant_id = tenant_id.as_str();
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    // Setup minimal balanced test data
    insert_test_account(
        &pool,
        tenant_id,
        "1000",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "3000",
        "Equity",
        AccountType::Equity,
        NormalBalance::Credit,
    )
    .await;

    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    insert_test_balance(&pool, tenant_id, period_id, "1000", "USD", 50000, 0).await; // $500 asset
    insert_test_balance(&pool, tenant_id, period_id, "3000", "USD", 0, 50000).await; // $500 equity

    // Make HTTP request (with JWT auth)
    let token = sign_test_jwt(tenant_id);
    let client = authed_client(&token);

    let url = format!(
        "{}/api/gl/balance-sheet?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, tenant_id, period_id
    );

    let response = client
        .get(&url)
        .send()
        .await
        .expect("Failed to fetch balance sheet");
    assert_eq!(response.status(), 200);

    // Parse as generic JSON to validate structure
    let json_value: serde_json::Value = response.json().await.expect("Failed to parse JSON");

    // Check top-level fields
    assert!(json_value.get("tenant_id").is_some());
    assert!(json_value.get("period_id").is_some());
    assert!(json_value.get("currency").is_some());
    assert!(json_value.get("rows").is_some());
    assert!(json_value.get("totals").is_some());

    // Check totals structure
    let totals = &json_value["totals"];
    assert!(totals.get("total_assets").is_some());
    assert!(totals.get("total_liabilities").is_some());
    assert!(totals.get("total_equity").is_some());
    assert!(totals.get("is_balanced").is_some());

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
