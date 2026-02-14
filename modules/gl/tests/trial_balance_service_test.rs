//! Trial Balance Service Integration Tests (Phase 14.4: bd-1cn)
//!
//! Tests for the refactored trial balance service using statement_repo.
//! Validates:
//! - Sorted by account code
//! - Totals must balance (sum debits == sum credits)
//! - Assertion fails if imbalance
//! - Deterministic JSON snapshot

use chrono::NaiveDate;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::trial_balance_service;
use sqlx::PgPool;
use uuid::Uuid;

mod common;

/// Test helper to create test data
async fn setup_test_data(pool: &PgPool, tenant_id: &str, period_id: Uuid) {
    // Create accounting period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, false, now())
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
    .bind(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap())
    .execute(pool)
    .await
    .unwrap();

    // Create accounts
    let accounts = vec![
        ("1000", "Cash", AccountType::Asset, NormalBalance::Debit),
        ("1100", "Accounts Receivable", AccountType::Asset, NormalBalance::Debit),
        ("2000", "Accounts Payable", AccountType::Liability, NormalBalance::Credit),
        ("4000", "Sales Revenue", AccountType::Revenue, NormalBalance::Credit),
        ("5000", "Operating Expenses", AccountType::Expense, NormalBalance::Debit),
    ];

    for (code, name, account_type, normal_balance) in accounts {
        sqlx::query(
            r#"
            INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, true, now())
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
        .unwrap();
    }

    // Create balanced account balances
    // Cash: $100 debit
    // A/R: $50 debit
    // A/P: $75 credit
    // Revenue: $100 credit
    // Expenses: $25 debit
    // Total debits: 100 + 50 + 25 = 175
    // Total credits: 75 + 100 = 175 (balanced!)

    let balances = vec![
        ("1000", 10000i64, 0i64),       // Cash: $100 debit
        ("1100", 5000, 0),               // A/R: $50 debit
        ("2000", 0, 7500),               // A/P: $75 credit
        ("4000", 0, 10000),              // Revenue: $100 credit
        ("5000", 2500, 0),               // Expenses: $25 debit
    ];

    for (account_code, debit_total, credit_total) in balances {
        let net_balance = debit_total - credit_total;
        sqlx::query(
            r#"
            INSERT INTO account_balances (
                tenant_id, account_code, period_id, currency,
                debit_total_minor, credit_total_minor, net_balance_minor,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, now(), now())
            ON CONFLICT (tenant_id, account_code, period_id, currency) DO UPDATE
            SET debit_total_minor = EXCLUDED.debit_total_minor,
                credit_total_minor = EXCLUDED.credit_total_minor,
                net_balance_minor = EXCLUDED.net_balance_minor,
                updated_at = now()
            "#,
        )
        .bind(tenant_id)
        .bind(account_code)
        .bind(period_id)
        .bind("USD")
        .bind(debit_total)
        .bind(credit_total)
        .bind(net_balance)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Test helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_trial_balance_service_balanced() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-tb-service-balanced";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;
    setup_test_data(&pool, tenant_id, period_id).await;

    // Execute
    let result = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        "USD",
    )
    .await;

    // Verify
    assert!(result.is_ok(), "Trial balance should succeed for balanced books");
    let response = result.unwrap();

    // Check metadata
    assert_eq!(response.tenant_id, tenant_id);
    assert_eq!(response.period_id, period_id);
    assert_eq!(response.currency, "USD");

    // Check rows are sorted by account code
    assert_eq!(response.rows.len(), 5);
    assert_eq!(response.rows[0].account_code, "1000");
    assert_eq!(response.rows[1].account_code, "1100");
    assert_eq!(response.rows[2].account_code, "2000");
    assert_eq!(response.rows[3].account_code, "4000");
    assert_eq!(response.rows[4].account_code, "5000");

    // Check totals are balanced
    assert_eq!(response.totals.total_debits, 17500); // 10000 + 5000 + 2500
    assert_eq!(response.totals.total_credits, 17500); // 7500 + 10000
    assert!(response.totals.is_balanced);

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_trial_balance_service_unbalanced_error() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-tb-service-unbalanced";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;
    setup_test_data(&pool, tenant_id, period_id).await;

    // Corrupt data: add extra debit to make it unbalanced
    sqlx::query(
        r#"
        UPDATE account_balances
        SET debit_total_minor = 15000, net_balance_minor = 15000
        WHERE tenant_id = $1 AND account_code = '1000'
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .unwrap();

    // Execute
    let result = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        "USD",
    )
    .await;

    // Verify: Should fail with Unbalanced error (acceptance criteria)
    assert!(result.is_err(), "Trial balance should fail for unbalanced books");
    let error = result.unwrap_err();

    match error {
        trial_balance_service::TrialBalanceError::Unbalanced { debits, credits } => {
            assert_eq!(debits, 22500); // 15000 + 5000 + 2500
            assert_eq!(credits, 17500); // 7500 + 10000
        }
        _ => panic!("Expected Unbalanced error, got: {:?}", error),
    }

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_trial_balance_service_deterministic_json_snapshot() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-tb-service-json";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;
    setup_test_data(&pool, tenant_id, period_id).await;

    // Execute twice
    let result1 = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        "USD",
    )
    .await
    .unwrap();

    let result2 = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        "USD",
    )
    .await
    .unwrap();

    // Serialize to JSON
    let json1 = serde_json::to_string_pretty(&result1).unwrap();
    let json2 = serde_json::to_string_pretty(&result2).unwrap();

    // Verify deterministic (same input → same output)
    assert_eq!(json1, json2, "JSON serialization must be deterministic");

    // Snapshot test: Verify JSON structure
    let json_value: serde_json::Value = serde_json::from_str(&json1).unwrap();

    // Check top-level fields
    assert!(json_value.get("tenant_id").is_some());
    assert!(json_value.get("period_id").is_some());
    assert!(json_value.get("currency").is_some());
    assert!(json_value.get("rows").is_some());
    assert!(json_value.get("totals").is_some());

    // Check rows array
    let rows = json_value["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);

    // Check first row structure (Cash account)
    let first_row = &rows[0];
    assert_eq!(first_row["account_code"], "1000");
    assert_eq!(first_row["account_name"], "Cash");
    assert_eq!(first_row["account_type"], "asset");
    assert_eq!(first_row["normal_balance"], "debit");
    assert_eq!(first_row["currency"], "USD");
    assert_eq!(first_row["debit_total_minor"], 10000);
    assert_eq!(first_row["credit_total_minor"], 0);
    assert_eq!(first_row["net_balance_minor"], 10000);

    // Check totals structure
    let totals = &json_value["totals"];
    assert_eq!(totals["total_debits"], 17500);
    assert_eq!(totals["total_credits"], 17500);
    assert_eq!(totals["is_balanced"], true);

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_trial_balance_service_period_not_found() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-tb-service-notfound";
    let non_existent_period_id = Uuid::new_v4();

    // Execute without creating period
    let result = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        non_existent_period_id,
        "USD",
    )
    .await;

    // Verify: Should fail with StatementRepo::PeriodNotFound error
    assert!(result.is_err(), "Trial balance should fail for non-existent period");
    let error = result.unwrap_err();

    match error {
        trial_balance_service::TrialBalanceError::StatementRepo(
            gl_rs::repos::statement_repo::StatementError::PeriodNotFound { .. }
        ) => {
            // Expected
        }
        _ => panic!("Expected PeriodNotFound error, got: {:?}", error),
    }
}

#[tokio::test]
async fn test_trial_balance_service_empty_period() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-tb-service-empty";
    let period_id = Uuid::new_v4();

    // Setup: Create period but no balances
    cleanup_test_data(&pool, tenant_id).await;

    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, false, now())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
    .bind(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap())
    .execute(&pool)
    .await
    .unwrap();

    // Execute
    let result = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        "USD",
    )
    .await;

    // Verify: Empty period should succeed with zero totals
    assert!(result.is_ok(), "Trial balance should succeed for empty period");
    let response = result.unwrap();

    assert_eq!(response.rows.len(), 0);
    assert_eq!(response.totals.total_debits, 0);
    assert_eq!(response.totals.total_credits, 0);
    assert!(response.totals.is_balanced, "Empty period should be balanced");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
