//! Integrated tests for payments consumer — real DB, no mocks.

use super::*;
use crate::domain::txns::service::is_event_processed;
use chrono::Utc;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_TENANT: &str = "test-treasury-payments-consumer";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    })
}

async fn test_pool() -> PgPool {
    PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to treasury test DB")
}

/// Insert a minimal active bank account, return its id.
async fn setup_bank_account(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        r#"
        INSERT INTO treasury_bank_accounts
            (app_id, account_name, currency, status)
        VALUES ($1, 'Test Checking', 'USD', 'active')
        RETURNING id
        "#,
    )
    .bind(TEST_TENANT)
    .fetch_one(pool)
    .await
    .expect("insert bank account failed")
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM processed_events WHERE processor = 'treasury:payments-consumer'")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
}

fn sample_payment_succeeded() -> PaymentSucceededPayload {
    PaymentSucceededPayload {
        payment_id: "pay-test-001".to_string(),
        invoice_id: "inv-test-001".to_string(),
        amount_minor: 50000,
        currency: "USD".to_string(),
    }
}

fn sample_ap_payment_executed(vendor_id: Uuid) -> ApPaymentExecutedPayload {
    ApPaymentExecutedPayload {
        payment_id: Uuid::new_v4(),
        run_id: Uuid::new_v4(),
        tenant_id: TEST_TENANT.to_string(),
        vendor_id,
        amount_minor: 120000,
        currency: "USD".to_string(),
        payment_method: "ach".to_string(),
        bank_reference: Some("ACH-REF-001".to_string()),
        executed_at: Utc::now(),
    }
}

#[tokio::test]
#[serial]
async fn test_payment_succeeded_creates_credit_txn() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let _acct_id = setup_bank_account(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = sample_payment_succeeded();

    let inserted = handle_payment_succeeded(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("handle failed");
    assert!(inserted, "expected row to be inserted");

    let (amount, external_id): (i64, String) = sqlx::query_as(
        "SELECT amount_minor, external_id FROM treasury_bank_transactions WHERE app_id = $1",
    )
    .bind(TEST_TENANT)
    .fetch_one(&pool)
    .await
    .expect("row not found");

    assert_eq!(amount, 50000, "credit should be positive");
    assert_eq!(external_id, event_id.to_string());

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_payment_succeeded_idempotent_on_replay() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let _acct_id = setup_bank_account(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = sample_payment_succeeded();

    handle_payment_succeeded(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("first call failed");
    let second = handle_payment_succeeded(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("second call must not error");

    assert!(!second, "replay must return false (duplicate skipped)");

    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(count, 1, "exactly one row after replay");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_payment_succeeded_no_account_skips_gracefully() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    // No bank account set up

    let event_id = Uuid::new_v4();
    let payload = sample_payment_succeeded();

    let inserted = handle_payment_succeeded(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("must not error when no account");
    assert!(!inserted, "no account → no txn row");

    // processed_events should still be recorded
    let processed = is_event_processed(&pool, event_id)
        .await
        .expect("check failed");
    assert!(
        processed,
        "event must be marked processed even without account"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_ap_payment_executed_creates_debit_txn() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let _acct_id = setup_bank_account(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = sample_ap_payment_executed(Uuid::new_v4());

    let inserted = handle_ap_payment_executed(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("handle failed");
    assert!(inserted, "expected row to be inserted");

    let (amount,): (i64,) =
        sqlx::query_as("SELECT amount_minor FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("row not found");

    assert_eq!(amount, -120000, "debit should be negative");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_ap_payment_executed_idempotent_on_replay() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let _acct_id = setup_bank_account(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = sample_ap_payment_executed(Uuid::new_v4());

    handle_ap_payment_executed(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("first call failed");
    let second = handle_ap_payment_executed(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("second call must not error");

    assert!(!second, "replay must return false");

    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(count, 1, "exactly one row after replay");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let _acct = setup_bank_account(&pool).await;

    let other_tenant = "other-tenant-payments-consumer";

    let event_id = Uuid::new_v4();
    let payload = sample_payment_succeeded();

    // Ingest for TEST_TENANT
    handle_payment_succeeded(&pool, event_id, TEST_TENANT, &payload)
        .await
        .expect("handle failed");

    // Other tenant sees no transactions
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(other_tenant)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(count, 0, "other tenant must see zero transactions");

    cleanup(&pool).await;
}
