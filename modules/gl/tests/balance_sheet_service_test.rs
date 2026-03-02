//! Balance Sheet Service Integration Tests (Phase 14.6: bd-12y)
//!
//! Tests for the balance sheet service using statement_repo.
//! Validates:
//! - Sorted by account type (asset, liability, equity), then account code
//! - Assets MUST equal Liabilities + Equity (fundamental accounting equation)
//! - Assertion fails if imbalance
//! - Deterministic JSON snapshot

use chrono::NaiveDate;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::balance_sheet_service;
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

    // Create accounts (Asset, Liability, Equity only for balance sheet)
    let accounts = vec![
        ("1000", "Cash", AccountType::Asset, NormalBalance::Debit),
        (
            "1100",
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
            "2100",
            "Notes Payable",
            AccountType::Liability,
            NormalBalance::Credit,
        ),
        (
            "3000",
            "Common Stock",
            AccountType::Equity,
            NormalBalance::Credit,
        ),
        (
            "3100",
            "Retained Earnings",
            AccountType::Equity,
            NormalBalance::Credit,
        ),
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

    // Create balanced balance sheet
    // Assets: Cash $100 + A/R $50 = $150 (total assets)
    // Liabilities: A/P $30 + Notes Payable $20 = $50 (total liabilities)
    // Equity: Common Stock $80 + Retained Earnings $20 = $100 (total equity)
    // Check: Assets (150) == Liabilities (50) + Equity (100) ✓

    let balances = vec![
        ("1000", 10000i64, 0i64), // Cash: $100 debit
        ("1100", 5000, 0),        // A/R: $50 debit
        ("2000", 0, 3000),        // A/P: $30 credit
        ("2100", 0, 2000),        // Notes Payable: $20 credit
        ("3000", 0, 8000),        // Common Stock: $80 credit
        ("3100", 0, 2000),        // Retained Earnings: $20 credit
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
async fn test_balance_sheet_service_balanced() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-bs-service-balanced";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;
    setup_test_data(&pool, tenant_id, period_id).await;

    // Execute
    let result = balance_sheet_service::get_balance_sheet(&pool, tenant_id, period_id, "USD").await;

    // Verify
    assert!(
        result.is_ok(),
        "Balance sheet should succeed for balanced books"
    );
    let response = result.unwrap();

    // Check metadata
    assert_eq!(response.tenant_id, tenant_id);
    assert_eq!(response.period_id, period_id);
    assert_eq!(response.currency, "USD");

    // Check rows are sorted by account type, then account code
    assert_eq!(response.rows.len(), 6);

    // Assets (1000, 1100)
    assert_eq!(response.rows[0].account_code, "1000");
    assert_eq!(response.rows[0].account_type, "asset");
    assert_eq!(response.rows[0].amount_minor, 10000);

    assert_eq!(response.rows[1].account_code, "1100");
    assert_eq!(response.rows[1].account_type, "asset");
    assert_eq!(response.rows[1].amount_minor, 5000);

    // Liabilities (2000, 2100)
    assert_eq!(response.rows[2].account_code, "2000");
    assert_eq!(response.rows[2].account_type, "liability");
    assert_eq!(response.rows[2].amount_minor, 3000);

    assert_eq!(response.rows[3].account_code, "2100");
    assert_eq!(response.rows[3].account_type, "liability");
    assert_eq!(response.rows[3].amount_minor, 2000);

    // Equity (3000, 3100)
    assert_eq!(response.rows[4].account_code, "3000");
    assert_eq!(response.rows[4].account_type, "equity");
    assert_eq!(response.rows[4].amount_minor, 8000);

    assert_eq!(response.rows[5].account_code, "3100");
    assert_eq!(response.rows[5].account_type, "equity");
    assert_eq!(response.rows[5].amount_minor, 2000);

    // Check totals satisfy accounting equation: Assets = Liabilities + Equity
    assert_eq!(response.totals.total_assets, 15000); // 10000 + 5000
    assert_eq!(response.totals.total_liabilities, 5000); // 3000 + 2000
    assert_eq!(response.totals.total_equity, 10000); // 8000 + 2000
    assert!(response.totals.is_balanced);
    assert_eq!(
        response.totals.total_assets,
        response.totals.total_liabilities + response.totals.total_equity,
        "Accounting equation must hold: Assets = Liabilities + Equity"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_balance_sheet_service_unbalanced_error() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-bs-service-unbalanced";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;
    setup_test_data(&pool, tenant_id, period_id).await;

    // Corrupt data: add extra asset to make it unbalanced
    sqlx::query(
        r#"
        UPDATE account_balances
        SET debit_total_minor = 20000, net_balance_minor = 20000
        WHERE tenant_id = $1 AND account_code = '1000'
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .unwrap();

    // Execute
    let result = balance_sheet_service::get_balance_sheet(&pool, tenant_id, period_id, "USD").await;

    // Verify: Should fail with Unbalanced error (acceptance criteria)
    assert!(
        result.is_err(),
        "Balance sheet should fail for unbalanced books"
    );
    let error = result.unwrap_err();

    match error {
        balance_sheet_service::BalanceSheetError::Unbalanced {
            assets,
            liabilities,
            equity,
        } => {
            assert_eq!(assets, 25000); // 20000 + 5000
            assert_eq!(liabilities, 5000); // 3000 + 2000
            assert_eq!(equity, 10000); // 8000 + 2000
                                       // Verify imbalance: 25000 != 5000 + 10000 (15000)
            assert_ne!(assets, liabilities + equity);
        }
        _ => panic!("Expected Unbalanced error, got: {:?}", error),
    }

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_balance_sheet_service_deterministic_json_snapshot() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-bs-service-json";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;
    setup_test_data(&pool, tenant_id, period_id).await;

    // Execute twice
    let result1 = balance_sheet_service::get_balance_sheet(&pool, tenant_id, period_id, "USD")
        .await
        .unwrap();

    let result2 = balance_sheet_service::get_balance_sheet(&pool, tenant_id, period_id, "USD")
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
    assert_eq!(rows.len(), 6);

    // Check first row structure (Cash account)
    let first_row = &rows[0];
    assert_eq!(first_row["account_code"], "1000");
    assert_eq!(first_row["account_name"], "Cash");
    assert_eq!(first_row["account_type"], "asset");
    assert_eq!(first_row["currency"], "USD");
    assert_eq!(first_row["amount_minor"], 10000);

    // Check totals structure
    let totals = &json_value["totals"];
    assert_eq!(totals["total_assets"], 15000);
    assert_eq!(totals["total_liabilities"], 5000);
    assert_eq!(totals["total_equity"], 10000);
    assert_eq!(totals["is_balanced"], true);

    // Verify accounting equation in JSON
    let assets = totals["total_assets"].as_i64().unwrap();
    let liabilities = totals["total_liabilities"].as_i64().unwrap();
    let equity = totals["total_equity"].as_i64().unwrap();
    assert_eq!(
        assets,
        liabilities + equity,
        "Accounting equation must hold in JSON"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_balance_sheet_service_period_not_found() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-bs-service-notfound";
    let non_existent_period_id = Uuid::new_v4();

    // Execute without creating period
    let result =
        balance_sheet_service::get_balance_sheet(&pool, tenant_id, non_existent_period_id, "USD")
            .await;

    // Verify: Should fail with StatementRepo::PeriodNotFound error
    assert!(
        result.is_err(),
        "Balance sheet should fail for non-existent period"
    );
    let error = result.unwrap_err();

    match error {
        balance_sheet_service::BalanceSheetError::StatementRepo(
            gl_rs::repos::statement_repo::StatementError::PeriodNotFound { .. },
        ) => {
            // Expected
        }
        _ => panic!("Expected PeriodNotFound error, got: {:?}", error),
    }
}

#[tokio::test]
async fn test_balance_sheet_service_empty_period() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-bs-service-empty";
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
    let result = balance_sheet_service::get_balance_sheet(&pool, tenant_id, period_id, "USD").await;

    // Verify: Empty period should succeed with zero totals
    assert!(
        result.is_ok(),
        "Balance sheet should succeed for empty period"
    );
    let response = result.unwrap();

    assert_eq!(response.rows.len(), 0);
    assert_eq!(response.totals.total_assets, 0);
    assert_eq!(response.totals.total_liabilities, 0);
    assert_eq!(response.totals.total_equity, 0);
    assert!(
        response.totals.is_balanced,
        "Empty period should be balanced (0 = 0 + 0)"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
async fn test_balance_sheet_service_negative_balances() {
    let pool = common::get_test_pool().await;
    let tenant_id = "test-bs-service-negative";
    let period_id = Uuid::new_v4();

    // Setup
    cleanup_test_data(&pool, tenant_id).await;

    // Create accounting period
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

    // Create accounts including contra account
    let accounts = vec![
        ("1000", "Cash", AccountType::Asset, NormalBalance::Debit),
        (
            "1900",
            "Allowance for Doubtful Accounts",
            AccountType::Asset,
            NormalBalance::Credit,
        ), // Contra asset
        (
            "2000",
            "Accounts Payable",
            AccountType::Liability,
            NormalBalance::Credit,
        ),
        (
            "3000",
            "Retained Earnings",
            AccountType::Equity,
            NormalBalance::Credit,
        ),
    ];

    for (code, name, account_type, normal_balance) in accounts {
        sqlx::query(
            r#"
            INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, true, now())
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(code)
        .bind(name)
        .bind(account_type)
        .bind(normal_balance)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Create balances with contra account (negative)
    // Assets: Cash $100 - Allowance $10 = $90 (net assets)
    // Liabilities: A/P $30
    // Equity: Retained Earnings $60
    // Check: Assets (90) == Liabilities (30) + Equity (60) ✓

    let balances = vec![
        ("1000", 10000i64, 0i64), // Cash: $100 debit
        ("1900", 0, 1000),        // Allowance: $10 credit (contra asset = negative net balance)
        ("2000", 0, 3000),        // A/P: $30 credit
        ("3000", 0, 6000),        // Retained Earnings: $60 credit
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
            "#,
        )
        .bind(tenant_id)
        .bind(account_code)
        .bind(period_id)
        .bind("USD")
        .bind(debit_total)
        .bind(credit_total)
        .bind(net_balance)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Execute
    let result = balance_sheet_service::get_balance_sheet(&pool, tenant_id, period_id, "USD").await;

    // Verify
    assert!(
        result.is_ok(),
        "Balance sheet should handle contra accounts"
    );
    let response = result.unwrap();

    // Check totals
    assert_eq!(response.totals.total_assets, 9000); // 10000 - 1000 (contra asset)
    assert_eq!(response.totals.total_liabilities, 3000);
    assert_eq!(response.totals.total_equity, 6000);
    assert!(response.totals.is_balanced);

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
