/// Trial Balance API E2E Test
///
/// This test verifies that:
/// 1. Trial balance API endpoint returns correct data
/// 2. Response includes account metadata (code, name, type, normal_balance)
/// 3. Totals are calculated correctly (total_debits, total_credits, is_balanced)
/// 4. Currency filtering works correctly
///
/// Run with: cargo test --test trial_balance_api_e2e -- --test-threads=1

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// API Response Types (matching OpenAPI contract)
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
struct TrialBalanceResponse {
    tenant_id: String,
    period_id: Uuid,
    currency: Option<String>,
    rows: Vec<TrialBalanceRow>,
    totals: TrialBalanceTotals,
}

#[derive(Debug, Deserialize, Serialize)]
struct TrialBalanceRow {
    account_code: String,
    account_name: String,
    account_type: String,
    normal_balance: String,
    currency: String,
    debit_total_minor: i64,
    credit_total_minor: i64,
    net_balance_minor: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct TrialBalanceTotals {
    total_debits: i64,
    total_credits: i64,
    is_balanced: bool,
}

// ============================================================================
// Test Infrastructure
// ============================================================================

async fn connect_gl_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://gl_user:gl_pass@localhost:5438/gl_db")
        .await
        .expect("Failed to connect to GL database")
}

async fn wait_for_service_healthy(container: &str, timeout_secs: u64) -> Result<(), String> {
    println!("⏳ Checking if {} is healthy...", container);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        let output = Command::new("docker")
            .args(&["inspect", "--format", "{{.State.Health.Status}}", container])
            .output()
            .map_err(|e| format!("Failed to inspect container: {}", e))?;

        let health = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if health == "healthy" {
            println!("✓ {} is healthy", container);
            return Ok(());
        }

        if tokio::time::Instant::now() > deadline {
            return Err(format!(
                "Timeout waiting for {} to be healthy (current status: {})",
                container, health
            ));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

async fn create_accounting_period(gl_pool: &PgPool, tenant_id: &str) -> Result<Uuid, String> {
    // Check if period already exists
    let existing: Option<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM accounting_periods
        WHERE tenant_id = $1
          AND period_start = $2
          AND period_end = $3
        "#,
    )
    .bind(tenant_id)
    .bind(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
    .bind(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())
    .fetch_optional(gl_pool)
    .await
    .map_err(|e| format!("Failed to check existing period: {}", e))?;

    if let Some((period_id,)) = existing {
        println!("✓ Using existing accounting period: {}", period_id);
        return Ok(period_id);
    }

    // Create new period
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed)
        VALUES ($1, $2, $3, $4, false)
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
    .bind(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create accounting period: {}", e))?;

    println!("✓ Created accounting period: {}", period_id);
    Ok(period_id)
}

async fn create_coa_accounts(gl_pool: &PgPool, tenant_id: &str) -> Result<(), String> {
    // Create Cash account (1000)
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES ($1, $2, '1000', 'Cash', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create Cash account: {}", e))?;

    // Create AR account (1100)
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES ($1, $2, '1100', 'Accounts Receivable', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create AR account: {}", e))?;

    // Create Revenue account (4000)
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES ($1, $2, '4000', 'Revenue', 'revenue', 'credit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create Revenue account: {}", e))?;

    println!("✓ Created COA accounts (1000, 1100, 4000)");
    Ok(())
}

async fn create_test_balances(
    gl_pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<(), String> {
    let entry_id = Uuid::new_v4();

    // Create balances for Cash: $1000 debit
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            id, tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor,
            last_journal_entry_id
        )
        VALUES ($1, $2, $3, '1000', 'USD', 100000, 0, 100000, $4)
        ON CONFLICT (tenant_id, period_id, account_code, currency) DO UPDATE
        SET debit_total_minor = account_balances.debit_total_minor + EXCLUDED.debit_total_minor,
            net_balance_minor = account_balances.net_balance_minor + EXCLUDED.net_balance_minor
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(entry_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create Cash balance: {}", e))?;

    // Create balances for AR: $500 debit
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            id, tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor,
            last_journal_entry_id
        )
        VALUES ($1, $2, $3, '1100', 'USD', 50000, 0, 50000, $4)
        ON CONFLICT (tenant_id, period_id, account_code, currency) DO UPDATE
        SET debit_total_minor = account_balances.debit_total_minor + EXCLUDED.debit_total_minor,
            net_balance_minor = account_balances.net_balance_minor + EXCLUDED.net_balance_minor
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(entry_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create AR balance: {}", e))?;

    // Create balances for Revenue: $1500 credit
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            id, tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor,
            last_journal_entry_id
        )
        VALUES ($1, $2, $3, '4000', 'USD', 0, 150000, -150000, $4)
        ON CONFLICT (tenant_id, period_id, account_code, currency) DO UPDATE
        SET credit_total_minor = account_balances.credit_total_minor + EXCLUDED.credit_total_minor,
            net_balance_minor = account_balances.net_balance_minor + EXCLUDED.net_balance_minor
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(entry_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create Revenue balance: {}", e))?;

    println!("✓ Created test balances (Cash: $1000, AR: $500, Revenue: $1500)");
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_trial_balance_api() {
    // Wait for GL service to be healthy
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy - ensure 'docker compose up -d' is running");

    // Connect to database
    let gl_pool = connect_gl_db().await;
    let tenant_id = "test_tenant_trial_balance";

    // Create test data
    let period_id = create_accounting_period(&gl_pool, tenant_id)
        .await
        .expect("Failed to create accounting period");

    create_coa_accounts(&gl_pool, tenant_id)
        .await
        .expect("Failed to create COA accounts");

    create_test_balances(&gl_pool, tenant_id, period_id)
        .await
        .expect("Failed to create test balances");

    // Call trial balance API
    let url = format!(
        "http://localhost:8090/api/gl/trial-balance?tenant_id={}&period_id={}",
        tenant_id, period_id
    );

    println!("📡 Calling trial balance API: {}", url);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .expect("Failed to call trial balance API");

    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK, got {}",
        response.status()
    );

    let tb: TrialBalanceResponse = response
        .json()
        .await
        .expect("Failed to parse trial balance response");

    // Verify response structure
    println!("✓ Trial balance API returned {} rows", tb.rows.len());
    assert_eq!(tb.tenant_id, tenant_id);
    assert_eq!(tb.period_id, period_id);
    assert_eq!(tb.currency, None); // No currency filter

    // Verify rows
    assert_eq!(tb.rows.len(), 3, "Expected 3 accounts");

    // Find accounts by code
    let cash = tb.rows.iter().find(|r| r.account_code == "1000").unwrap();
    let ar = tb.rows.iter().find(|r| r.account_code == "1100").unwrap();
    let revenue = tb.rows.iter().find(|r| r.account_code == "4000").unwrap();

    // Verify Cash account
    assert_eq!(cash.account_name, "Cash");
    assert_eq!(cash.account_type, "asset");
    assert_eq!(cash.normal_balance, "debit");
    assert_eq!(cash.debit_total_minor, 100000); // $1000.00
    assert_eq!(cash.credit_total_minor, 0);
    assert_eq!(cash.net_balance_minor, 100000);

    // Verify AR account
    assert_eq!(ar.account_name, "Accounts Receivable");
    assert_eq!(ar.account_type, "asset");
    assert_eq!(ar.normal_balance, "debit");
    assert_eq!(ar.debit_total_minor, 50000); // $500.00
    assert_eq!(ar.credit_total_minor, 0);
    assert_eq!(ar.net_balance_minor, 50000);

    // Verify Revenue account
    assert_eq!(revenue.account_name, "Revenue");
    assert_eq!(revenue.account_type, "revenue");
    assert_eq!(revenue.normal_balance, "credit");
    assert_eq!(revenue.debit_total_minor, 0);
    assert_eq!(revenue.credit_total_minor, 150000); // $1500.00
    assert_eq!(revenue.net_balance_minor, -150000);

    // Verify totals
    assert_eq!(tb.totals.total_debits, 150000); // $1000 + $500
    assert_eq!(tb.totals.total_credits, 150000); // $1500
    assert!(tb.totals.is_balanced, "Trial balance should be balanced");

    println!("✅ Trial balance API test passed!");
}

#[tokio::test]
async fn test_trial_balance_api_with_currency_filter() {
    // Wait for GL service to be healthy
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy");

    let gl_pool = connect_gl_db().await;
    let tenant_id = "test_tenant_trial_balance_currency";

    // Create test data
    let period_id = create_accounting_period(&gl_pool, tenant_id)
        .await
        .expect("Failed to create accounting period");

    create_coa_accounts(&gl_pool, tenant_id)
        .await
        .expect("Failed to create COA accounts");

    create_test_balances(&gl_pool, tenant_id, period_id)
        .await
        .expect("Failed to create test balances");

    // Call trial balance API with currency filter
    let url = format!(
        "http://localhost:8090/api/gl/trial-balance?tenant_id={}&period_id={}&currency=USD",
        tenant_id, period_id
    );

    println!("📡 Calling trial balance API with currency filter: {}", url);

    let client = reqwest::Client::new();
    let response = client.get(&url).send().await.expect("Failed to call API");

    assert_eq!(response.status(), 200);

    let tb: TrialBalanceResponse = response.json().await.expect("Failed to parse response");

    // Verify currency filter is reflected in response
    assert_eq!(tb.currency, Some("USD".to_string()));

    // Verify all rows have USD currency
    for row in &tb.rows {
        assert_eq!(row.currency, "USD");
    }

    println!("✅ Trial balance API with currency filter test passed!");
}
