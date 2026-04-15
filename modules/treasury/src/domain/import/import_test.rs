//! Integrated tests for the statement import service.

use super::*;
use crate::domain::accounts::{service as acct_svc, CreateBankAccountRequest};
use serial_test::serial;

const TEST_APP: &str = "test-app-import";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    })
}

async fn test_pool() -> PgPool {
    sqlx::PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to treasury test database")
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_statements WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'bank_statement' \
         OR aggregate_type = 'bank_account'",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
}

fn sample_csv() -> Vec<u8> {
    b"date,description,amount,reference\n\
      2024-01-15,Coffee Shop,-4.50,TXN001\n\
      2024-01-16,Salary,5000.00,SAL001\n\
      2024-01-17,Groceries,-82.30,TXN002\n"
        .to_vec()
}

async fn create_test_account(pool: &PgPool) -> Uuid {
    let req = CreateBankAccountRequest {
        account_name: "Import Test Account".to_string(),
        institution: Some("Test Bank".to_string()),
        account_number_last4: Some("9999".to_string()),
        routing_number: None,
        currency: "USD".to_string(),
        metadata: None,
    };
    let account = acct_svc::create_bank_account(pool, TEST_APP, &req, None, "test".to_string())
        .await
        .expect("create test account");
    account.id
}

#[tokio::test]
#[serial]
async fn test_import_creates_statement_and_lines() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_account(&pool).await;

    let req = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 100000,
        closing_balance_minor: 591320,
        csv_data: sample_csv(),
        filename: Some("jan-2024.csv".to_string()),
        format: None,
    };

    let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
        .await
        .expect("import failed");

    assert_eq!(result.lines_imported, 3);
    assert_eq!(result.lines_skipped, 0);
    assert!(result.errors.is_empty());

    // Verify statement exists
    let stmt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_statements WHERE id = $1 AND app_id = $2",
    )
    .bind(result.statement_id)
    .bind(TEST_APP)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stmt_count, 1);

    // Verify transaction lines
    let txn_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_transactions WHERE statement_id = $1 AND app_id = $2",
    )
    .bind(result.statement_id)
    .bind(TEST_APP)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(txn_count, 3);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_reimport_same_file_is_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_account(&pool).await;

    let csv = sample_csv();
    let req1 = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 100000,
        closing_balance_minor: 591320,
        csv_data: csv.clone(),
        filename: Some("jan-2024.csv".to_string()),
        format: None,
    };

    let first = import_statement(&pool, TEST_APP, req1, "c1".to_string())
        .await
        .expect("first import failed");

    let req2 = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 100000,
        closing_balance_minor: 591320,
        csv_data: csv,
        filename: Some("jan-2024.csv".to_string()),
        format: None,
    };

    let second = import_statement(&pool, TEST_APP, req2, "c2".to_string()).await;
    assert!(
        matches!(second, Err(ImportError::DuplicateImport { statement_id }) if statement_id == first.statement_id),
        "expected DuplicateImport, got {:?}",
        second
    );

    // Verify no extra lines were created
    let txn_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(TEST_APP)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(txn_count, 3);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_import_invalid_rows_reported() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_account(&pool).await;

    let csv = b"date,description,amount\n\
                 2024-01-15,Valid,-10.00\n\
                 bad-date,Invalid Date,20.00\n\
                 2024-01-17,,30.00\n\
                 2024-01-18,Also Valid,-5.00\n";

    let req = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: csv.to_vec(),
        filename: None,
        format: None,
    };

    let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
        .await
        .expect("import failed");

    assert_eq!(result.lines_imported, 2);
    assert_eq!(result.errors.len(), 2);
    assert_eq!(result.errors[0].line, 3); // bad date
    assert_eq!(result.errors[1].line, 4); // empty description

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_import_nonexistent_account() {
    let pool = test_pool().await;
    let fake_id = Uuid::new_v4();

    let req = ImportRequest {
        account_id: fake_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: sample_csv(),
        filename: None,
        format: None,
    };

    let result = import_statement(&pool, TEST_APP, req, "c1".to_string()).await;
    assert!(matches!(result, Err(ImportError::AccountNotFound(_))));
}

// ================================================================
// CC adapter integration tests
// ================================================================

async fn create_test_cc_account(pool: &PgPool) -> Uuid {
    use crate::domain::accounts::{service as acct_svc, CreateCreditCardAccountRequest};
    let req = CreateCreditCardAccountRequest {
        account_name: "CC Import Test".to_string(),
        institution: Some("Test Issuer".to_string()),
        account_number_last4: Some("5555".to_string()),
        currency: "USD".to_string(),
        credit_limit_minor: Some(500_000),
        statement_closing_day: Some(15),
        cc_network: Some("Visa".to_string()),
        metadata: None,
    };
    acct_svc::create_credit_card_account(pool, TEST_APP, &req, None, "test".to_string())
        .await
        .expect("create CC test account")
        .id
}

#[tokio::test]
#[serial]
async fn test_import_chase_cc_statement() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_cc_account(&pool).await;

    let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
                 01/15/2024,01/16/2024,STARBUCKS,Food & Drink,Sale,-4.50,\n\
                 01/18/2024,01/19/2024,AMAZON.COM,Shopping,Sale,-89.99,\n\
                 01/20/2024,01/20/2024,PAYMENT,,Payment,250.00,\n";

    let req = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: csv.to_vec(),
        filename: Some("chase-jan-2024.csv".to_string()),
        format: Some(CsvFormat::ChaseCredit),
    };

    let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
        .await
        .expect("Chase CC import failed");

    assert_eq!(result.lines_imported, 3);
    assert!(result.errors.is_empty());

    // Verify amounts stored correctly
    let amounts: Vec<(i64,)> = sqlx::query_as(
        "SELECT amount_minor FROM treasury_bank_transactions \
         WHERE statement_id = $1 AND app_id = $2 ORDER BY transaction_date",
    )
    .bind(result.statement_id)
    .bind(TEST_APP)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(amounts[0].0, -450); // charge
    assert_eq!(amounts[1].0, -8999); // charge
    assert_eq!(amounts[2].0, 25000); // payment

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_import_amex_cc_statement() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_cc_account(&pool).await;

    // Amex: charges positive, credits negative
    let csv = b"Date,Description,Amount\n\
                 01/15/2024,STARBUCKS,4.50\n\
                 01/18/2024,AMAZON.COM,89.99\n\
                 01/20/2024,PAYMENT RECEIVED,-250.00\n";

    let req = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: csv.to_vec(),
        filename: Some("amex-jan-2024.csv".to_string()),
        format: Some(CsvFormat::AmexCredit),
    };

    let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
        .await
        .expect("Amex CC import failed");

    assert_eq!(result.lines_imported, 3);
    assert!(result.errors.is_empty());

    // Verify sign normalisation: Amex positive charges -> negative stored
    let amounts: Vec<(i64,)> = sqlx::query_as(
        "SELECT amount_minor FROM treasury_bank_transactions \
         WHERE statement_id = $1 AND app_id = $2 ORDER BY transaction_date",
    )
    .bind(result.statement_id)
    .bind(TEST_APP)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(amounts[0].0, -450); // charge (was +4.50)
    assert_eq!(amounts[1].0, -8999); // charge (was +89.99)
    assert_eq!(amounts[2].0, 25000); // payment (was -250.00)

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_cc_reimport_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_cc_account(&pool).await;

    let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount\n\
                 01/15/2024,01/16/2024,STARBUCKS,Food,Sale,-4.50\n";

    let req1 = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: csv.to_vec(),
        filename: None,
        format: Some(CsvFormat::ChaseCredit),
    };

    let first = import_statement(&pool, TEST_APP, req1, "c1".to_string())
        .await
        .expect("first CC import failed");

    let req2 = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: csv.to_vec(),
        filename: None,
        format: Some(CsvFormat::ChaseCredit),
    };

    let second = import_statement(&pool, TEST_APP, req2, "c2".to_string()).await;
    assert!(
        matches!(second, Err(ImportError::DuplicateImport { statement_id }) if statement_id == first.statement_id),
        "expected DuplicateImport, got {:?}",
        second
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_auto_detect_chase_format() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let account_id = create_test_cc_account(&pool).await;

    // No format specified -- should auto-detect Chase from headers
    let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount\n\
                 01/15/2024,01/16/2024,STARBUCKS,Food,Sale,-4.50\n";

    let req = ImportRequest {
        account_id,
        period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        opening_balance_minor: 0,
        closing_balance_minor: 0,
        csv_data: csv.to_vec(),
        filename: None,
        format: None, // auto-detect
    };

    let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
        .await
        .expect("auto-detect Chase import failed");

    assert_eq!(result.lines_imported, 1);
    assert!(result.errors.is_empty());

    cleanup(&pool).await;
}
