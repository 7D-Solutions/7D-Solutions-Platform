use chrono::{NaiveDate, Utc};
use gl_rs::db::init_pool;
use gl_rs::repos::balance_repo::{self, AccountBalance};
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

/// Helper to create a test accounting period
async fn insert_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    is_closed: bool,
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
    .bind(is_closed)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test period");

    period_id
}

/// Helper to cleanup test balances
async fn cleanup_balances(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup balances");
}

/// Helper to cleanup test period
async fn cleanup_period(pool: &PgPool, period_id: Uuid) {
    sqlx::query("DELETE FROM accounting_periods WHERE id = $1")
        .bind(period_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup period");
}

#[tokio::test]
#[serial]
async fn test_tx_upsert_rollup_insert_new_balance() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-001";
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        false,
    )
    .await;

    let account_code = "1000";
    let currency = "USD";
    let journal_entry_id = Uuid::new_v4();

    // Start transaction
    let mut tx = pool.begin().await.expect("Failed to start transaction");

    // Insert new balance
    let balance = balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        account_code,
        currency,
        10000, // $100.00 debit
        5000,  // $50.00 credit
        journal_entry_id,
    )
    .await
    .expect("Failed to upsert balance");

    tx.commit().await.expect("Failed to commit transaction");

    // Verify the balance
    assert_eq!(balance.tenant_id, tenant_id);
    assert_eq!(balance.period_id, period_id);
    assert_eq!(balance.account_code, account_code);
    assert_eq!(balance.currency, currency);
    assert_eq!(balance.debit_total_minor, 10000);
    assert_eq!(balance.credit_total_minor, 5000);
    assert_eq!(balance.net_balance_minor, 5000); // 10000 - 5000
    assert_eq!(balance.last_journal_entry_id, Some(journal_entry_id));

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period_id).await;
}

#[tokio::test]
#[serial]
async fn test_tx_upsert_rollup_increments_existing_balance() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-002";
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        false,
    )
    .await;

    let account_code = "2000";
    let currency = "USD";
    let entry1 = Uuid::new_v4();
    let entry2 = Uuid::new_v4();

    // First upsert
    let mut tx1 = pool.begin().await.expect("Failed to start transaction");
    let balance1 = balance_repo::tx_upsert_rollup(
        &mut tx1,
        tenant_id,
        period_id,
        account_code,
        currency,
        10000, // $100.00 debit
        0,
        entry1,
    )
    .await
    .expect("Failed to upsert balance");
    tx1.commit().await.expect("Failed to commit");

    assert_eq!(balance1.debit_total_minor, 10000);
    assert_eq!(balance1.credit_total_minor, 0);
    assert_eq!(balance1.net_balance_minor, 10000);

    // Second upsert (increment)
    let mut tx2 = pool.begin().await.expect("Failed to start transaction");
    let balance2 = balance_repo::tx_upsert_rollup(
        &mut tx2,
        tenant_id,
        period_id,
        account_code,
        currency,
        5000,  // +$50.00 debit
        20000, // +$200.00 credit
        entry2,
    )
    .await
    .expect("Failed to upsert balance");
    tx2.commit().await.expect("Failed to commit");

    // Verify cumulative totals
    assert_eq!(balance2.debit_total_minor, 15000); // 10000 + 5000
    assert_eq!(balance2.credit_total_minor, 20000); // 0 + 20000
    assert_eq!(balance2.net_balance_minor, -5000); // 15000 - 20000
    assert_eq!(balance2.last_journal_entry_id, Some(entry2));

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period_id).await;
}

#[tokio::test]
#[serial]
async fn test_tx_upsert_rollup_respects_unique_grain() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-003";
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        false,
    )
    .await;

    let account_code = "3000";
    let entry_id = Uuid::new_v4();

    // Create balance for USD
    let mut tx1 = pool.begin().await.expect("Failed to start transaction");
    let balance_usd = balance_repo::tx_upsert_rollup(
        &mut tx1,
        tenant_id,
        period_id,
        account_code,
        "USD",
        10000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert USD balance");
    tx1.commit().await.expect("Failed to commit");

    // Create balance for EUR (different currency = different grain)
    let mut tx2 = pool.begin().await.expect("Failed to start transaction");
    let balance_eur = balance_repo::tx_upsert_rollup(
        &mut tx2,
        tenant_id,
        period_id,
        account_code,
        "EUR",
        20000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert EUR balance");
    tx2.commit().await.expect("Failed to commit");

    // Verify separate balances
    assert_ne!(balance_usd.id, balance_eur.id);
    assert_eq!(balance_usd.currency, "USD");
    assert_eq!(balance_usd.debit_total_minor, 10000);
    assert_eq!(balance_eur.currency, "EUR");
    assert_eq!(balance_eur.debit_total_minor, 20000);

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_by_grain_success() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-004";
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 4, 30).unwrap(),
        false,
    )
    .await;

    let account_code = "4000";
    let currency = "GBP";
    let entry_id = Uuid::new_v4();

    // Create balance
    let mut tx = pool.begin().await.expect("Failed to start transaction");
    balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        account_code,
        currency,
        30000,
        10000,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");
    tx.commit().await.expect("Failed to commit");

    // Find by grain
    let result = balance_repo::find_by_grain(&pool, tenant_id, period_id, account_code, currency)
        .await
        .expect("Failed to find balance");

    assert!(result.is_some(), "Balance should be found");
    let balance = result.unwrap();
    assert_eq!(balance.tenant_id, tenant_id);
    assert_eq!(balance.period_id, period_id);
    assert_eq!(balance.account_code, account_code);
    assert_eq!(balance.currency, currency);
    assert_eq!(balance.debit_total_minor, 30000);
    assert_eq!(balance.credit_total_minor, 10000);
    assert_eq!(balance.net_balance_minor, 20000);

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_by_grain_not_found() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-999";
    let period_id = Uuid::new_v4();
    let account_code = "9999";
    let currency = "ZZZ";

    // Try to find non-existent balance
    let result = balance_repo::find_by_grain(&pool, tenant_id, period_id, account_code, currency)
        .await
        .expect("Query should succeed");

    assert!(result.is_none(), "Balance should not be found");
}

#[tokio::test]
#[serial]
async fn test_find_trial_balance_all_currencies() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-005";
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 5, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 5, 31).unwrap(),
        false,
    )
    .await;

    let entry_id = Uuid::new_v4();

    // Create multiple balances
    let mut tx = pool.begin().await.expect("Failed to start transaction");

    balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        "1000",
        "USD",
        50000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");

    balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        "2000",
        "USD",
        0,
        30000,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");

    balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        "1000",
        "EUR",
        20000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");

    tx.commit().await.expect("Failed to commit");

    // Query trial balance (all currencies)
    let balances = balance_repo::find_trial_balance(&pool, tenant_id, period_id, None)
        .await
        .expect("Failed to find trial balance");

    assert_eq!(balances.len(), 3, "Should have 3 balances");

    // Verify ordering (by account_code, currency)
    assert_eq!(balances[0].account_code, "1000");
    assert_eq!(balances[0].currency, "EUR");
    assert_eq!(balances[1].account_code, "1000");
    assert_eq!(balances[1].currency, "USD");
    assert_eq!(balances[2].account_code, "2000");
    assert_eq!(balances[2].currency, "USD");

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_trial_balance_single_currency() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-006";
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
        false,
    )
    .await;

    let entry_id = Uuid::new_v4();

    // Create balances in multiple currencies
    let mut tx = pool.begin().await.expect("Failed to start transaction");

    balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        "1000",
        "USD",
        100000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");

    balance_repo::tx_upsert_rollup(
        &mut tx,
        tenant_id,
        period_id,
        "2000",
        "EUR",
        50000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");

    tx.commit().await.expect("Failed to commit");

    // Query trial balance for USD only
    let balances = balance_repo::find_trial_balance(&pool, tenant_id, period_id, Some("USD"))
        .await
        .expect("Failed to find trial balance");

    assert_eq!(balances.len(), 1, "Should have 1 USD balance");
    assert_eq!(balances[0].currency, "USD");
    assert_eq!(balances[0].account_code, "1000");

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_balance_history() {
    let pool = setup_test_pool().await;

    let tenant_id = "tenant-balance-007";

    // Create two periods
    let period1_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
        false,
    )
    .await;

    let period2_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        false,
    )
    .await;

    let account_code = "1000";
    let currency = "USD";
    let entry_id = Uuid::new_v4();

    // Create balance for period 1
    let mut tx1 = pool.begin().await.expect("Failed to start transaction");
    balance_repo::tx_upsert_rollup(
        &mut tx1,
        tenant_id,
        period1_id,
        account_code,
        currency,
        10000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");
    tx1.commit().await.expect("Failed to commit");

    // Create balance for period 2
    let mut tx2 = pool.begin().await.expect("Failed to start transaction");
    balance_repo::tx_upsert_rollup(
        &mut tx2,
        tenant_id,
        period2_id,
        account_code,
        currency,
        20000,
        0,
        entry_id,
    )
    .await
    .expect("Failed to upsert balance");
    tx2.commit().await.expect("Failed to commit");

    // Query balance history
    let history = balance_repo::find_balance_history(&pool, tenant_id, account_code, Some(currency))
        .await
        .expect("Failed to find balance history");

    assert_eq!(history.len(), 2, "Should have 2 periods");

    // Verify ordering (most recent first)
    assert_eq!(history[0].period_id, period2_id);
    assert_eq!(history[0].debit_total_minor, 20000);
    assert_eq!(history[1].period_id, period1_id);
    assert_eq!(history[1].debit_total_minor, 10000);

    // Cleanup
    cleanup_balances(&pool, tenant_id).await;
    cleanup_period(&pool, period1_id).await;
    cleanup_period(&pool, period2_id).await;
}
