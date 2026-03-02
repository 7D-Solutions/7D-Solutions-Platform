//! Treasury account CRUD service — DB operations with Guard→Mutation→Outbox atomicity.
//!
//! Supports both bank accounts (account_type='bank') and credit card accounts
//! (account_type='credit_card'). Write operations follow:
//! 1. Guard: validate inputs, check preconditions, check idempotency
//! 2. Mutation: write to treasury_bank_accounts
//! 3. Outbox: enqueue event atomically in same transaction

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::outbox::enqueue_event_tx;

use super::{
    AccountError, AccountStatus, CreateBankAccountRequest, CreateCreditCardAccountRequest,
    TreasuryAccount, UpdateAccountRequest,
};

const EVT_ACCOUNT_CREATED: &str = "bank_account.created";
const EVT_ACCOUNT_UPDATED: &str = "bank_account.updated";
const EVT_ACCOUNT_DEACTIVATED: &str = "bank_account.deactivated";

// ============================================================================
// Reads
// ============================================================================

pub async fn get_account(
    pool: &PgPool,
    app_id: &str,
    id: Uuid,
) -> Result<Option<TreasuryAccount>, AccountError> {
    let account = sqlx::query_as::<_, TreasuryAccount>(
        r#"
        SELECT id, app_id, account_name, account_type, institution, account_number_last4,
               routing_number, currency, current_balance_minor, status,
               credit_limit_minor, statement_closing_day, cc_network,
               metadata, created_at, updated_at
        FROM treasury_bank_accounts
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(account)
}

pub async fn list_accounts(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
) -> Result<Vec<TreasuryAccount>, AccountError> {
    let accounts = if include_inactive {
        sqlx::query_as::<_, TreasuryAccount>(
            r#"
            SELECT id, app_id, account_name, account_type, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status,
                   credit_limit_minor, statement_closing_day, cc_network,
                   metadata, created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1
            ORDER BY account_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, TreasuryAccount>(
            r#"
            SELECT id, app_id, account_name, account_type, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status,
                   credit_limit_minor, statement_closing_day, cc_network,
                   metadata, created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1 AND status = 'active'::treasury_account_status
            ORDER BY account_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await?
    };

    Ok(accounts)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a bank account. Supports idempotency via `idempotency_key`.
///
/// If `idempotency_key` is Some and has been used before, returns
/// `AccountError::IdempotentReplay` with the cached response.
pub async fn create_bank_account(
    pool: &PgPool,
    app_id: &str,
    req: &CreateBankAccountRequest,
    idempotency_key: Option<&str>,
    correlation_id: String,
) -> Result<TreasuryAccount, AccountError> {
    req.validate()?;

    if let Some(key) = idempotency_key {
        check_idempotency(pool, app_id, key).await?;
    }

    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = req.currency.to_uppercase();

    let mut tx = pool.begin().await?;

    let account = sqlx::query_as::<_, TreasuryAccount>(
        r#"
        INSERT INTO treasury_bank_accounts (
            id, app_id, account_name, account_type, institution, account_number_last4,
            routing_number, currency, current_balance_minor, status,
            credit_limit_minor, statement_closing_day, cc_network, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'bank'::treasury_account_type, $4, $5, $6, $7, 0,
                'active'::treasury_account_status, NULL, NULL, NULL, $8, $9, $9)
        RETURNING id, app_id, account_name, account_type, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status,
                  credit_limit_minor, statement_closing_day, cc_network,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(req.account_name.trim())
    .bind(&req.institution)
    .bind(&req.account_number_last4)
    .bind(&req.routing_number)
    .bind(&currency)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    let payload = serde_json::json!({
        "account_id": id,
        "app_id": app_id,
        "account_name": account.account_name,
        "account_type": "bank",
        "currency": account.currency,
        "status": "active",
        "correlation_id": correlation_id,
        "created_at": account.created_at,
    });

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ACCOUNT_CREATED,
        "bank_account",
        &id.to_string(),
        &payload,
    )
    .await?;

    record_idempotency(&mut tx, app_id, idempotency_key, &account, 201, now).await?;

    tx.commit().await?;

    Ok(account)
}

/// Create a credit card account. Supports idempotency via `idempotency_key`.
pub async fn create_credit_card_account(
    pool: &PgPool,
    app_id: &str,
    req: &CreateCreditCardAccountRequest,
    idempotency_key: Option<&str>,
    correlation_id: String,
) -> Result<TreasuryAccount, AccountError> {
    req.validate()?;

    if let Some(key) = idempotency_key {
        check_idempotency(pool, app_id, key).await?;
    }

    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = req.currency.to_uppercase();

    let mut tx = pool.begin().await?;

    let account = sqlx::query_as::<_, TreasuryAccount>(
        r#"
        INSERT INTO treasury_bank_accounts (
            id, app_id, account_name, account_type, institution, account_number_last4,
            routing_number, currency, current_balance_minor, status,
            credit_limit_minor, statement_closing_day, cc_network, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'credit_card'::treasury_account_type, $4, $5, NULL, $6, 0,
                'active'::treasury_account_status, $7, $8, $9, $10, $11, $11)
        RETURNING id, app_id, account_name, account_type, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status,
                  credit_limit_minor, statement_closing_day, cc_network,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(req.account_name.trim())
    .bind(&req.institution)
    .bind(&req.account_number_last4)
    .bind(&currency)
    .bind(req.credit_limit_minor)
    .bind(req.statement_closing_day)
    .bind(&req.cc_network)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    let payload = serde_json::json!({
        "account_id": id,
        "app_id": app_id,
        "account_name": account.account_name,
        "account_type": "credit_card",
        "currency": account.currency,
        "status": "active",
        "correlation_id": correlation_id,
        "created_at": account.created_at,
    });

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ACCOUNT_CREATED,
        "bank_account",
        &id.to_string(),
        &payload,
    )
    .await?;

    record_idempotency(&mut tx, app_id, idempotency_key, &account, 201, now).await?;

    tx.commit().await?;

    Ok(account)
}

/// Update mutable account fields. Emits `bank_account.updated` via outbox.
pub async fn update_account(
    pool: &PgPool,
    app_id: &str,
    id: Uuid,
    req: &UpdateAccountRequest,
    correlation_id: String,
) -> Result<TreasuryAccount, AccountError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    let existing: Option<TreasuryAccount> = sqlx::query_as(
        r#"
        SELECT id, app_id, account_name, account_type, institution, account_number_last4,
               routing_number, currency, current_balance_minor, status,
               credit_limit_minor, statement_closing_day, cc_network,
               metadata, created_at, updated_at
        FROM treasury_bank_accounts
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(AccountError::NotFound(id))?;

    let new_name = req
        .account_name
        .as_deref()
        .map(str::trim)
        .map(String::from)
        .unwrap_or(current.account_name.clone());
    let new_institution = req.institution.clone().or(current.institution.clone());
    let new_last4 = req
        .account_number_last4
        .clone()
        .or(current.account_number_last4.clone());
    let new_routing = req
        .routing_number
        .clone()
        .or(current.routing_number.clone());
    let new_limit = req.credit_limit_minor.or(current.credit_limit_minor);
    let new_closing = req.statement_closing_day.or(current.statement_closing_day);
    let new_network = req.cc_network.clone().or(current.cc_network.clone());
    let new_metadata = req.metadata.clone().or(current.metadata.clone());

    let account = sqlx::query_as::<_, TreasuryAccount>(
        r#"
        UPDATE treasury_bank_accounts
        SET account_name = $1, institution = $2, account_number_last4 = $3,
            routing_number = $4, credit_limit_minor = $5, statement_closing_day = $6,
            cc_network = $7, metadata = $8, updated_at = $9
        WHERE id = $10 AND app_id = $11
        RETURNING id, app_id, account_name, account_type, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status,
                  credit_limit_minor, statement_closing_day, cc_network,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(&new_name)
    .bind(&new_institution)
    .bind(&new_last4)
    .bind(&new_routing)
    .bind(new_limit)
    .bind(new_closing)
    .bind(&new_network)
    .bind(&new_metadata)
    .bind(now)
    .bind(id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    let payload = serde_json::json!({
        "account_id": id,
        "app_id": app_id,
        "account_name": account.account_name,
        "correlation_id": correlation_id,
        "updated_at": now,
    });

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ACCOUNT_UPDATED,
        "bank_account",
        &id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    Ok(account)
}

/// Deactivate an account (soft delete). Emits `bank_account.deactivated`.
pub async fn deactivate_account(
    pool: &PgPool,
    app_id: &str,
    id: Uuid,
    actor: &str,
    correlation_id: String,
) -> Result<(), AccountError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    let exists: Option<(AccountStatus,)> =
        sqlx::query_as("SELECT status FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2")
            .bind(id)
            .bind(app_id)
            .fetch_optional(&mut *tx)
            .await?;

    if exists.is_none() {
        return Err(AccountError::NotFound(id));
    }

    sqlx::query(
        r#"
        UPDATE treasury_bank_accounts
        SET status = 'inactive'::treasury_account_status, updated_at = $1
        WHERE id = $2 AND app_id = $3
        "#,
    )
    .bind(now)
    .bind(id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

    let payload = serde_json::json!({
        "account_id": id,
        "app_id": app_id,
        "actor": actor,
        "correlation_id": correlation_id,
        "deactivated_at": now,
    });

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ACCOUNT_DEACTIVATED,
        "bank_account",
        &id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}

// ============================================================================
// Idempotency helpers
// ============================================================================

async fn check_idempotency(pool: &PgPool, app_id: &str, key: &str) -> Result<(), AccountError> {
    let cached: Option<(serde_json::Value, i32)> = sqlx::query_as(
        "SELECT response_body, status_code FROM treasury_idempotency_keys WHERE app_id = $1 AND idempotency_key = $2 LIMIT 1",
    )
    .bind(app_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;

    if let Some((body, code)) = cached {
        return Err(AccountError::IdempotentReplay {
            status_code: code as u16,
            body,
        });
    }

    Ok(())
}

async fn record_idempotency(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    idempotency_key: Option<&str>,
    account: &TreasuryAccount,
    status_code: i32,
    now: chrono::DateTime<Utc>,
) -> Result<(), AccountError> {
    if let Some(key) = idempotency_key {
        let response_body = serde_json::to_value(account).unwrap_or(serde_json::Value::Null);
        let expires_at = now + chrono::Duration::hours(24);
        sqlx::query(
            r#"
            INSERT INTO treasury_idempotency_keys
                (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
            VALUES ($1, $2, '', $3, $4, $5)
            ON CONFLICT (app_id, idempotency_key) DO NOTHING
            "#,
        )
        .bind(app_id)
        .bind(key)
        .bind(response_body)
        .bind(status_code)
        .bind(expires_at)
        .execute(&mut **tx)
        .await
        .map_err(AccountError::Database)?;
    }
    Ok(())
}

// ============================================================================
// Integrated Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::super::AccountType;
    use super::*;
    use serial_test::serial;

    const TEST_APP: &str = "test-app-accounts";

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
        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'bank_account' AND aggregate_id IN \
             (SELECT id::TEXT FROM treasury_bank_accounts WHERE app_id = $1)",
        )
        .bind(TEST_APP)
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

    fn sample_bank() -> CreateBankAccountRequest {
        CreateBankAccountRequest {
            account_name: "Main Checking".to_string(),
            institution: Some("First National".to_string()),
            account_number_last4: Some("4321".to_string()),
            routing_number: Some("021000021".to_string()),
            currency: "USD".to_string(),
            metadata: None,
        }
    }

    fn sample_cc() -> CreateCreditCardAccountRequest {
        CreateCreditCardAccountRequest {
            account_name: "Corp Visa".to_string(),
            institution: Some("Chase".to_string()),
            account_number_last4: Some("9876".to_string()),
            currency: "USD".to_string(),
            credit_limit_minor: Some(500_000),
            statement_closing_day: Some(15),
            cc_network: Some("Visa".to_string()),
            metadata: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_create_bank_account() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let account = create_bank_account(&pool, TEST_APP, &sample_bank(), None, "c1".to_string())
            .await
            .expect("create bank account failed");

        assert_eq!(account.account_name, "Main Checking");
        assert_eq!(account.account_type, AccountType::Bank);
        assert_eq!(account.currency, "USD");
        assert_eq!(account.status, AccountStatus::Active);
        assert_eq!(account.current_balance_minor, 0);
        assert!(account.credit_limit_minor.is_none());

        let fetched = get_account(&pool, TEST_APP, account.id)
            .await
            .expect("get failed");
        assert!(fetched.is_some());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_create_credit_card_account() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let account =
            create_credit_card_account(&pool, TEST_APP, &sample_cc(), None, "c1".to_string())
                .await
                .expect("create CC failed");

        assert_eq!(account.account_type, AccountType::CreditCard);
        assert_eq!(account.credit_limit_minor, Some(500_000));
        assert_eq!(account.statement_closing_day, Some(15));
        assert_eq!(account.cc_network.as_deref(), Some("Visa"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_list_accounts_active_only() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let a1 = create_bank_account(&pool, TEST_APP, &sample_bank(), None, "c1".to_string())
            .await
            .expect("create 1 failed");
        let mut req2 = sample_bank();
        req2.account_name = "Savings".to_string();
        let a2 = create_bank_account(&pool, TEST_APP, &req2, None, "c2".to_string())
            .await
            .expect("create 2 failed");

        deactivate_account(&pool, TEST_APP, a2.id, "system", "c3".to_string())
            .await
            .expect("deactivate failed");

        let active = list_accounts(&pool, TEST_APP, false)
            .await
            .expect("list failed");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, a1.id);

        let all = list_accounts(&pool, TEST_APP, true)
            .await
            .expect("list all failed");
        assert_eq!(all.len(), 2);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_update_account() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_bank_account(&pool, TEST_APP, &sample_bank(), None, "c1".to_string())
            .await
            .expect("create failed");

        let updated = update_account(
            &pool,
            TEST_APP,
            created.id,
            &UpdateAccountRequest {
                account_name: Some("New Name".to_string()),
                institution: None,
                account_number_last4: None,
                routing_number: None,
                credit_limit_minor: None,
                statement_closing_day: None,
                cc_network: None,
                metadata: None,
            },
            "c2".to_string(),
        )
        .await
        .expect("update failed");

        assert_eq!(updated.account_name, "New Name");
        assert_eq!(updated.institution.as_deref(), Some("First National"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_create() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let key = Some("idem-key-001");
        create_bank_account(&pool, TEST_APP, &sample_bank(), key, "c1".to_string())
            .await
            .expect("first create failed");

        let result =
            create_bank_account(&pool, TEST_APP, &sample_bank(), key, "c2".to_string()).await;
        assert!(
            matches!(result, Err(AccountError::IdempotentReplay { .. })),
            "expected IdempotentReplay, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_wrong_app_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_bank_account(&pool, TEST_APP, &sample_bank(), None, "c1".to_string())
            .await
            .expect("create failed");

        let result = get_account(&pool, "other-app", created.id)
            .await
            .expect("get failed");
        assert!(result.is_none());

        cleanup(&pool).await;
    }
}
