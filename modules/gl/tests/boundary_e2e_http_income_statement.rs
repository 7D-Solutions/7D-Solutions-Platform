//! Boundary E2E Test: HTTP → Router → Service → DB (Income Statement Read Path)
//!
//! This test validates the REAL ingress boundary for GL income statement queries:
//! 1. Makes actual HTTP GET request to `/api/gl/income-statement`
//! 2. Validates response shape, serialization, status codes
//! 3. Tests error handling (400 for missing params)
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
use gl_rs::services::income_statement_service::IncomeStatementResponse;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// JWT Auth Helpers (GL service requires Bearer JWT)
// ============================================================================

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

fn sign_test_jwt(tenant_id: &str) -> String {
    dotenvy::dotenv().ok();
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set (loaded from .env)");
    let encoding_key =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("Invalid JWT_PRIVATE_KEY_PEM");
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
    jsonwebtoken::encode(&header, &claims, &encoding_key).expect("Failed to sign test JWT")
}

fn authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token)
            .parse()
            .expect("valid header value"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("Failed to build authed client")
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

/// Helper to directly insert balances for testing income statement endpoint
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
async fn test_boundary_http_income_statement_returns_correct_json() {
    // Setup
    let pool = setup_test_pool().await;
    let tenant_id = Uuid::new_v4().to_string();
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // JWT auth
    let token = sign_test_jwt(&tenant_id);
    let client = authed_client(&token);

    // Cleanup
    cleanup_test_data(&pool, &tenant_id).await;

    // Setup Chart of Accounts
    insert_test_account(
        &pool,
        &tenant_id,
        "4000",
        "Sales Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    insert_test_account(
        &pool,
        &tenant_id,
        "5000",
        "Operating Expenses",
        AccountType::Expense,
        NormalBalance::Debit,
    )
    .await;

    // Setup accounting period
    let period_id = insert_test_period(
        &pool,
        &tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).expect("valid date"),
        NaiveDate::from_ymd_opt(2024, 2, 29).expect("valid date"),
    )
    .await;

    // Insert test balances (Revenue: $1000 credit, Expenses: $400 debit)
    insert_test_balance(&pool, &tenant_id, period_id, "4000", "USD", 0, 100000).await; // $1000 revenue
    insert_test_balance(&pool, &tenant_id, period_id, "5000", "USD", 40000, 0).await; // $400 expenses

    // BOUNDARY TEST: Make real HTTP GET request with JWT auth
    let url = format!(
        "{}/api/gl/income-statement?tenant_id={}&period_id={}&currency=USD",
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
        "Expected 200 OK from income statement endpoint"
    );

    // Assert: Response is valid JSON matching IncomeStatementResponse structure
    let income_statement: IncomeStatementResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Correct tenant_id and period_id in response
    assert_eq!(income_statement.tenant_id, tenant_id);
    assert_eq!(income_statement.period_id, period_id);
    assert_eq!(income_statement.currency, "USD");

    // Assert: Rows present and correct totals
    assert_eq!(
        income_statement.rows.len(),
        2,
        "Should have 2 account rows (revenue + expense)"
    );

    // Assert: Totals calculation (Net Income = Revenue + Expenses = $1000 + (-$400) = $600)
    // Sign convention: Revenue is positive, Expenses are negative
    assert_eq!(
        income_statement.totals.total_revenue, 100000,
        "Revenue should be $1000 (100000 minor units)"
    );
    assert_eq!(
        income_statement.totals.total_expenses, -40000,
        "Expenses should be -$400 (-40000 minor units, negative sign)"
    );
    assert_eq!(
        income_statement.totals.net_income, 60000,
        "Net income should be $600 (60000 minor units = 100000 + (-40000))"
    );

    // Cleanup
    cleanup_test_data(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_boundary_http_income_statement_error_handling() {
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // JWT auth for authenticated requests
    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_test_jwt(&tenant_id);
    let client = authed_client(&token);

    // Test: Missing required query parameter (should return 400)
    let url_missing_params = format!("{}/api/gl/income-statement", gl_service_url);

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
        "{}/api/gl/income-statement?tenant_id={}&period_id=not-a-uuid&currency=USD",
        gl_service_url, tenant_id
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
async fn test_boundary_http_income_statement_schema_validation() {
    // This test verifies the JSON schema matches expectations (deterministic serialization)
    let pool = setup_test_pool().await;
    let tenant_id = Uuid::new_v4().to_string();
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    // JWT auth
    let token = sign_test_jwt(&tenant_id);
    let client = authed_client(&token);

    // Cleanup
    cleanup_test_data(&pool, &tenant_id).await;

    // Setup minimal test data
    insert_test_account(
        &pool,
        &tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    let period_id = insert_test_period(
        &pool,
        &tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).expect("valid date"),
        NaiveDate::from_ymd_opt(2024, 3, 31).expect("valid date"),
    )
    .await;

    insert_test_balance(&pool, &tenant_id, period_id, "4000", "USD", 0, 50000).await;

    // Make authenticated HTTP request
    let url = format!(
        "{}/api/gl/income-statement?tenant_id={}&period_id={}&currency=USD",
        gl_service_url, tenant_id, period_id
    );

    let response = client
        .get(&url)
        .send()
        .await
        .expect("Failed to fetch income statement");
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
    assert!(totals.get("total_revenue").is_some());
    assert!(totals.get("total_expenses").is_some());
    assert!(totals.get("net_income").is_some());

    // Cleanup
    cleanup_test_data(&pool, &tenant_id).await;
}
