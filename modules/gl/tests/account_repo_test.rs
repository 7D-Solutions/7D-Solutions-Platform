use chrono::Utc;
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{self, AccountError, AccountType, NormalBalance};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5438/gl_test".to_string());

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
    is_active: bool,
) -> Uuid {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .bind(is_active)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test account");

    id
}

/// Helper to cleanup test accounts
async fn cleanup_account(pool: &PgPool, id: Uuid) {
    sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("Failed to cleanup account");
}

#[tokio::test]
#[serial]
async fn test_find_by_code_success() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-001",
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Find the account
    let result = account_repo::find_by_code(&pool, "tenant-001", "1200")
        .await
        .expect("Failed to find account");

    assert!(result.is_some(), "Account should be found");
    let account = result.unwrap();
    assert_eq!(account.id, account_id);
    assert_eq!(account.tenant_id, "tenant-001");
    assert_eq!(account.code, "1200");
    assert_eq!(account.name, "Accounts Receivable");
    assert_eq!(account.account_type, AccountType::Asset);
    assert_eq!(account.normal_balance, NormalBalance::Debit);
    assert!(account.is_active);

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_by_code_not_found() {
    let pool = setup_test_pool().await;

    // Try to find a non-existent account
    let result = account_repo::find_by_code(&pool, "tenant-999", "9999")
        .await
        .expect("Query should succeed");

    assert!(result.is_none(), "Non-existent account should return None");
}

#[tokio::test]
#[serial]
async fn test_find_active_by_code_success() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-002",
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
        true,
    )
    .await;

    // Find the active account
    let account = account_repo::find_active_by_code(&pool, "tenant-002", "4000")
        .await
        .expect("Failed to find active account");

    assert_eq!(account.id, account_id);
    assert_eq!(account.code, "4000");
    assert!(account.is_active);

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_active_by_code_inactive() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-003",
        "5000",
        "Inactive Expense",
        AccountType::Expense,
        NormalBalance::Debit,
        false,
    )
    .await;

    // Try to find the inactive account
    let result = account_repo::find_active_by_code(&pool, "tenant-003", "5000").await;

    assert!(result.is_err(), "Should return error for inactive account");
    match result {
        Err(AccountError::Inactive { tenant_id, code }) => {
            assert_eq!(tenant_id, "tenant-003");
            assert_eq!(code, "5000");
        }
        _ => panic!("Expected Inactive error"),
    }

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_active_by_code_not_found() {
    let pool = setup_test_pool().await;

    // Try to find a non-existent account
    let result = account_repo::find_active_by_code(&pool, "tenant-888", "8888").await;

    assert!(result.is_err(), "Should return error for non-existent account");
    match result {
        Err(AccountError::NotFound { tenant_id, code }) => {
            assert_eq!(tenant_id, "tenant-888");
            assert_eq!(code, "8888");
        }
        _ => panic!("Expected NotFound error"),
    }
}

#[tokio::test]
#[serial]
async fn test_assert_active_success() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-004",
        "2000",
        "Accounts Payable",
        AccountType::Liability,
        NormalBalance::Credit,
        true,
    )
    .await;

    // Assert the account is active
    let result = account_repo::assert_active(&pool, "tenant-004", "2000").await;
    assert!(result.is_ok(), "Assert should succeed for active account");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_assert_active_fails_for_inactive() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-005",
        "3000",
        "Inactive Equity",
        AccountType::Equity,
        NormalBalance::Credit,
        false,
    )
    .await;

    // Assert should fail for inactive account
    let result = account_repo::assert_active(&pool, "tenant-005", "3000").await;
    assert!(result.is_err(), "Assert should fail for inactive account");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_transaction_variants() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-006",
        "1000",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Test find_by_code_tx
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    let result = account_repo::find_by_code_tx(&mut tx, "tenant-006", "1000")
        .await
        .expect("Failed to find account in transaction");

    assert!(result.is_some(), "Account should be found in transaction");

    // Test find_active_by_code_tx
    let account = account_repo::find_active_by_code_tx(&mut tx, "tenant-006", "1000")
        .await
        .expect("Failed to find active account in transaction");

    assert_eq!(account.id, account_id);

    // Test assert_active_tx
    let assert_result = account_repo::assert_active_tx(&mut tx, "tenant-006", "1000").await;
    assert!(assert_result.is_ok(), "Assert should succeed in transaction");

    tx.commit().await.expect("Failed to commit transaction");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_unique_tenant_code_constraint() {
    let pool = setup_test_pool().await;

    let account_id = insert_test_account(
        &pool,
        "tenant-007",
        "1100",
        "First Account",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Try to insert another account with the same tenant_id and code
    let result = sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind("tenant-007")
    .bind("1100") // Same code
    .bind("Duplicate Account")
    .bind(AccountType::Asset)
    .bind(NormalBalance::Debit)
    .bind(true)
    .bind(Utc::now())
    .execute(&pool)
    .await;

    assert!(result.is_err(), "Duplicate (tenant_id, code) should fail");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
#[serial]
async fn test_different_tenants_same_code() {
    let pool = setup_test_pool().await;

    // Insert account for tenant-008
    let account1_id = insert_test_account(
        &pool,
        "tenant-008",
        "1300",
        "Tenant 8 Account",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Insert account for tenant-009 with same code (should succeed)
    let account2_id = insert_test_account(
        &pool,
        "tenant-009",
        "1300",
        "Tenant 9 Account",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Both accounts should exist
    let account1 = account_repo::find_by_code(&pool, "tenant-008", "1300")
        .await
        .expect("Failed to find tenant-008 account")
        .expect("Tenant-008 account should exist");

    let account2 = account_repo::find_by_code(&pool, "tenant-009", "1300")
        .await
        .expect("Failed to find tenant-009 account")
        .expect("Tenant-009 account should exist");

    assert_ne!(account1.id, account2.id);
    assert_eq!(account1.code, account2.code);
    assert_eq!(account1.code, "1300");

    cleanup_account(&pool, account1_id).await;
    cleanup_account(&pool, account2_id).await;
}
