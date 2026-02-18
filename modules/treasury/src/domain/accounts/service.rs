//! Bank account CRUD service — DB operations with Guard→Mutation→Outbox atomicity.
//!
//! Write operations follow:
//! 1. Guard: validate inputs, check preconditions
//! 2. Mutation: write to treasury_bank_accounts
//! 3. Outbox: enqueue event atomically in same transaction

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::outbox::enqueue_event_tx;

use super::{AccountError, AccountStatus, BankAccount, CreateAccountRequest, UpdateAccountRequest};

// Event type constants
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
) -> Result<Option<BankAccount>, AccountError> {
    let account = sqlx::query_as::<_, BankAccount>(
        r#"
        SELECT id, app_id, account_name, institution, account_number_last4,
               routing_number, currency, current_balance_minor, status, metadata,
               created_at, updated_at
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
) -> Result<Vec<BankAccount>, AccountError> {
    let accounts = if include_inactive {
        sqlx::query_as::<_, BankAccount>(
            r#"
            SELECT id, app_id, account_name, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status, metadata,
                   created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1
            ORDER BY account_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, BankAccount>(
            r#"
            SELECT id, app_id, account_name, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status, metadata,
                   created_at, updated_at
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
pub async fn create_account(
    pool: &PgPool,
    app_id: &str,
    req: &CreateAccountRequest,
    idempotency_key: Option<&str>,
    correlation_id: String,
) -> Result<BankAccount, AccountError> {
    req.validate()?;

    // Guard: check idempotency key
    if let Some(key) = idempotency_key {
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
    }

    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = req.currency.to_uppercase();

    let mut tx = pool.begin().await?;

    // Mutation: insert account
    let account = sqlx::query_as::<_, BankAccount>(
        r#"
        INSERT INTO treasury_bank_accounts (
            id, app_id, account_name, institution, account_number_last4,
            routing_number, currency, current_balance_minor, status, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'active'::treasury_account_status, $8, $9, $9)
        RETURNING id, app_id, account_name, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status, metadata,
                  created_at, updated_at
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

    // Outbox: enqueue created event
    let payload = serde_json::json!({
        "account_id": id,
        "app_id": app_id,
        "account_name": account.account_name,
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

    // Idempotency: record key with 24h TTL
    if let Some(key) = idempotency_key {
        let response_body = serde_json::to_value(&account).unwrap_or(serde_json::Value::Null);
        let expires_at = now + chrono::Duration::hours(24);
        sqlx::query(
            r#"
            INSERT INTO treasury_idempotency_keys (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
            VALUES ($1, $2, $3, $4, 201, $5)
            ON CONFLICT (app_id, idempotency_key) DO NOTHING
            "#,
        )
        .bind(app_id)
        .bind(key)
        .bind("") // hash not enforced — key uniqueness is sufficient guard
        .bind(response_body)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
    }

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
) -> Result<BankAccount, AccountError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: account must exist for this app
    let existing: Option<BankAccount> = sqlx::query_as(
        r#"
        SELECT id, app_id, account_name, institution, account_number_last4,
               routing_number, currency, current_balance_minor, status, metadata,
               created_at, updated_at
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

    // Resolve updates (keep existing where not provided)
    let new_name = req
        .account_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or(current.account_name.clone());
    let new_institution = if req.institution.is_some() {
        req.institution.clone()
    } else {
        current.institution.clone()
    };
    let new_last4 = if req.account_number_last4.is_some() {
        req.account_number_last4.clone()
    } else {
        current.account_number_last4.clone()
    };
    let new_routing = if req.routing_number.is_some() {
        req.routing_number.clone()
    } else {
        current.routing_number.clone()
    };
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata.clone()
    };

    // Mutation
    let account = sqlx::query_as::<_, BankAccount>(
        r#"
        UPDATE treasury_bank_accounts
        SET account_name = $1, institution = $2, account_number_last4 = $3,
            routing_number = $4, metadata = $5, updated_at = $6
        WHERE id = $7 AND app_id = $8
        RETURNING id, app_id, account_name, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(&new_name)
    .bind(&new_institution)
    .bind(&new_last4)
    .bind(&new_routing)
    .bind(&new_metadata)
    .bind(now)
    .bind(id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: enqueue updated event
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

/// Deactivate a bank account (soft delete). Emits `bank_account.deactivated`.
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

    // Guard: account must exist
    let exists: Option<(AccountStatus,)> = sqlx::query_as(
        "SELECT status FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2",
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    if exists.is_none() {
        return Err(AccountError::NotFound(id));
    }

    // Mutation
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

    // Outbox: enqueue deactivated event
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
// Integrated Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_APP: &str = "test-app-accounts";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5444/treasury_db".to_string())
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

    fn sample_create() -> CreateAccountRequest {
        CreateAccountRequest {
            account_name: "Main Checking".to_string(),
            institution: Some("First National".to_string()),
            account_number_last4: Some("4321".to_string()),
            routing_number: Some("021000021".to_string()),
            currency: "USD".to_string(),
            metadata: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_create_and_get_account() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let account = create_account(&pool, TEST_APP, &sample_create(), None, "corr-1".to_string())
            .await
            .expect("create failed");

        assert_eq!(account.account_name, "Main Checking");
        assert_eq!(account.app_id, TEST_APP);
        assert_eq!(account.currency, "USD");
        assert_eq!(account.status, AccountStatus::Active);
        assert_eq!(account.current_balance_minor, 0);

        let fetched = get_account(&pool, TEST_APP, account.id)
            .await
            .expect("get failed");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, account.id);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_list_accounts_active_only() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let a1 = create_account(&pool, TEST_APP, &sample_create(), None, "c1".to_string())
            .await
            .expect("create 1 failed");
        let mut req2 = sample_create();
        req2.account_name = "Savings".to_string();
        let a2 = create_account(&pool, TEST_APP, &req2, None, "c2".to_string())
            .await
            .expect("create 2 failed");

        deactivate_account(&pool, TEST_APP, a2.id, "system", "c3".to_string())
            .await
            .expect("deactivate failed");

        let active = list_accounts(&pool, TEST_APP, false).await.expect("list failed");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, a1.id);

        let all = list_accounts(&pool, TEST_APP, true).await.expect("list all failed");
        assert_eq!(all.len(), 2);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_update_account() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_account(&pool, TEST_APP, &sample_create(), None, "c1".to_string())
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
                metadata: None,
            },
            "c2".to_string(),
        )
        .await
        .expect("update failed");

        assert_eq!(updated.account_name, "New Name");
        assert_eq!(updated.institution.as_deref(), Some("First National")); // unchanged

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_create() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let key = Some("idem-key-001");

        create_account(&pool, TEST_APP, &sample_create(), key, "c1".to_string())
            .await
            .expect("first create failed");

        let result = create_account(&pool, TEST_APP, &sample_create(), key, "c2".to_string()).await;
        assert!(
            matches!(result, Err(AccountError::IdempotentReplay { .. })),
            "expected IdempotentReplay, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_deactivate_emits_outbox_event() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_account(&pool, TEST_APP, &sample_create(), None, "c1".to_string())
            .await
            .expect("create failed");

        deactivate_account(&pool, TEST_APP, created.id, "user-1", "c2".to_string())
            .await
            .expect("deactivate failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bank_account' AND aggregate_id = $1",
        )
        .bind(created.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert!(count.0 >= 2, "expected created + deactivated events");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_wrong_app_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_account(&pool, TEST_APP, &sample_create(), None, "c1".to_string())
            .await
            .expect("create failed");

        let result = get_account(&pool, "other-app", created.id)
            .await
            .expect("get failed");
        assert!(result.is_none());

        cleanup(&pool).await;
    }
}
