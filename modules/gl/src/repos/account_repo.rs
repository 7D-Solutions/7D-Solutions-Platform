use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

/// Account type enum matching database account_type
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "account_type", rename_all = "lowercase")]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Revenue,
    Expense,
}

/// Normal balance enum matching database normal_balance
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "normal_balance", rename_all = "lowercase")]
pub enum NormalBalance {
    Debit,
    Credit,
}

/// Account model representing a Chart of Accounts entry
#[derive(Debug, Clone, FromRow)]
pub struct Account {
    pub id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    #[sqlx(rename = "type")]
    pub account_type: AccountType,
    pub normal_balance: NormalBalance,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Errors that can occur during account repository operations
#[derive(Debug, Error)]
pub enum AccountError {
    #[error("Account not found: tenant_id={tenant_id}, code={code}")]
    NotFound { tenant_id: String, code: String },

    #[error("Account is inactive: tenant_id={tenant_id}, code={code}")]
    Inactive { tenant_id: String, code: String },

    #[error("Account already exists: tenant_id={tenant_id}, code={code}")]
    Conflict { tenant_id: String, code: String },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Find an account by tenant_id and code
/// Returns None if account doesn't exist
pub async fn find_by_code(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
) -> Result<Option<Account>, AccountError> {
    let account = sqlx::query_as::<_, Account>(
        r#"
        SELECT id, tenant_id, code, name, type, normal_balance, is_active, created_at
        FROM accounts
        WHERE tenant_id = $1 AND code = $2
        "#,
    )
    .bind(tenant_id)
    .bind(code)
    .fetch_optional(pool)
    .await?;

    Ok(account)
}

/// Find an account by tenant_id and code within a transaction
pub async fn find_by_code_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    code: &str,
) -> Result<Option<Account>, AccountError> {
    let account = sqlx::query_as::<_, Account>(
        r#"
        SELECT id, tenant_id, code, name, type, normal_balance, is_active, created_at
        FROM accounts
        WHERE tenant_id = $1 AND code = $2
        "#,
    )
    .bind(tenant_id)
    .bind(code)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(account)
}

/// Find an active account by tenant_id and code
/// Returns error if account doesn't exist or is inactive
pub async fn find_active_by_code(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
) -> Result<Account, AccountError> {
    let account = find_by_code(pool, tenant_id, code).await?;

    match account {
        Some(acc) if acc.is_active => Ok(acc),
        Some(_) => Err(AccountError::Inactive {
            tenant_id: tenant_id.to_string(),
            code: code.to_string(),
        }),
        None => Err(AccountError::NotFound {
            tenant_id: tenant_id.to_string(),
            code: code.to_string(),
        }),
    }
}

/// Find an active account by tenant_id and code within a transaction
/// Returns error if account doesn't exist or is inactive
pub async fn find_active_by_code_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    code: &str,
) -> Result<Account, AccountError> {
    let account = find_by_code_tx(tx, tenant_id, code).await?;

    match account {
        Some(acc) if acc.is_active => Ok(acc),
        Some(_) => Err(AccountError::Inactive {
            tenant_id: tenant_id.to_string(),
            code: code.to_string(),
        }),
        None => Err(AccountError::NotFound {
            tenant_id: tenant_id.to_string(),
            code: code.to_string(),
        }),
    }
}

/// Assert that an account exists and is active
/// This is a convenience function for validation
pub async fn assert_active(pool: &PgPool, tenant_id: &str, code: &str) -> Result<(), AccountError> {
    find_active_by_code(pool, tenant_id, code).await?;
    Ok(())
}

/// Assert that an account exists and is active within a transaction
pub async fn assert_active_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    code: &str,
) -> Result<(), AccountError> {
    find_active_by_code_tx(tx, tenant_id, code).await?;
    Ok(())
}

/// Create a new account. Returns the account on success.
/// If an account with the same (tenant_id, code) already exists, returns
/// `AccountError::Conflict`.
pub async fn create_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: AccountType,
    normal_balance: NormalBalance,
) -> Result<Account, AccountError> {
    let id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let result = sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, true, $7)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(&account_type)
    .bind(&normal_balance)
    .bind(now)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AccountError::Conflict {
            tenant_id: tenant_id.to_string(),
            code: code.to_string(),
        });
    }

    Ok(Account {
        id,
        tenant_id: tenant_id.to_string(),
        code: code.to_string(),
        name: name.to_string(),
        account_type,
        normal_balance,
        is_active: true,
        created_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that AccountType enum matches database enum values
    #[test]
    fn test_account_type_variants() {
        // These should match the database enum values
        let types = vec![
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Equity,
            AccountType::Revenue,
            AccountType::Expense,
        ];
        assert_eq!(types.len(), 5);
    }

    /// Test that NormalBalance enum matches database enum values
    #[test]
    fn test_normal_balance_variants() {
        // These should match the database enum values
        let balances = vec![NormalBalance::Debit, NormalBalance::Credit];
        assert_eq!(balances.len(), 2);
    }
}
