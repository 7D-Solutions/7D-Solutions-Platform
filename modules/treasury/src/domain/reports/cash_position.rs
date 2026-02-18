//! Cash position query — real-time balance by account and currency.
//!
//! Computes position from:
//! - Opening balance: earliest statement's `opening_balance_minor` per account (0 if none)
//! - Transaction total: SUM of all `bank_transactions.amount_minor` per account
//! - Balance = opening + transaction total
//!
//! Results are separated into bank cash vs credit-card liability buckets.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::accounts::AccountType;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct AccountPositionRow {
    account_id: Uuid,
    account_name: String,
    account_type: AccountType,
    currency: String,
    institution: Option<String>,
    opening_balance_minor: i64,
    transaction_total_minor: i64,
}

/// Position for a single account.
#[derive(Debug, Clone, Serialize)]
pub struct AccountPosition {
    pub account_id: Uuid,
    pub account_name: String,
    pub currency: String,
    pub institution: Option<String>,
    pub opening_balance_minor: i64,
    pub transaction_total_minor: i64,
    pub balance_minor: i64,
}

/// Aggregated summary across all accounts.
#[derive(Debug, Clone, Serialize)]
pub struct CashPositionSummary {
    pub total_bank_cash_minor: i64,
    pub total_cc_liability_minor: i64,
    pub net_position_minor: i64,
    pub currencies: Vec<String>,
}

/// Full cash position response.
#[derive(Debug, Clone, Serialize)]
pub struct CashPositionResponse {
    pub as_of: DateTime<Utc>,
    pub bank_cash: Vec<AccountPosition>,
    pub credit_card_liability: Vec<AccountPosition>,
    pub summary: CashPositionSummary,
}

// ============================================================================
// Query
// ============================================================================

pub async fn get_cash_position(
    pool: &PgPool,
    app_id: &str,
) -> Result<CashPositionResponse, sqlx::Error> {
    let rows = sqlx::query_as::<_, AccountPositionRow>(
        r#"
        SELECT
            a.id                          AS account_id,
            a.account_name,
            a.account_type,
            a.currency,
            a.institution,
            COALESCE(
                (SELECT s.opening_balance_minor
                 FROM treasury_bank_statements s
                 WHERE s.account_id = a.id AND s.app_id = $1
                 ORDER BY s.period_start ASC
                 LIMIT 1),
                0
            )                             AS opening_balance_minor,
            COALESCE(SUM(t.amount_minor), 0)::BIGINT AS transaction_total_minor
        FROM treasury_bank_accounts a
        LEFT JOIN treasury_bank_transactions t
            ON t.account_id = a.id AND t.app_id = $1
        WHERE a.app_id = $1
            AND a.status = 'active'::treasury_account_status
        GROUP BY a.id, a.account_name, a.account_type, a.currency, a.institution
        ORDER BY a.account_type, a.account_name
        "#,
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let mut bank_cash = Vec::new();
    let mut cc_liability = Vec::new();
    let mut currencies = std::collections::BTreeSet::new();
    let mut total_bank: i64 = 0;
    let mut total_cc: i64 = 0;

    for row in rows {
        let balance = row.opening_balance_minor + row.transaction_total_minor;
        currencies.insert(row.currency.clone());

        let pos = AccountPosition {
            account_id: row.account_id,
            account_name: row.account_name,
            currency: row.currency,
            institution: row.institution,
            opening_balance_minor: row.opening_balance_minor,
            transaction_total_minor: row.transaction_total_minor,
            balance_minor: balance,
        };

        match row.account_type {
            AccountType::Bank => {
                total_bank += balance;
                bank_cash.push(pos);
            }
            AccountType::CreditCard => {
                total_cc += balance;
                cc_liability.push(pos);
            }
        }
    }

    Ok(CashPositionResponse {
        as_of: Utc::now(),
        bank_cash,
        credit_card_liability: cc_liability,
        summary: CashPositionSummary {
            total_bank_cash_minor: total_bank,
            total_cc_liability_minor: total_cc,
            net_position_minor: total_bank + total_cc,
            currencies: currencies.into_iter().collect(),
        },
    })
}

// ============================================================================
// Integrated Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{
        service as account_svc, CreateBankAccountRequest, CreateCreditCardAccountRequest,
    };
    use serial_test::serial;

    const TEST_APP: &str = "test-app-cash-pos";

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
        // Delete in dependency order
        sqlx::query("DELETE FROM treasury_recon_matches WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
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

    #[tokio::test]
    #[serial]
    async fn cash_position_empty_returns_zero_summary() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let pos = get_cash_position(&pool, TEST_APP)
            .await
            .expect("query failed");

        assert!(pos.bank_cash.is_empty());
        assert!(pos.credit_card_liability.is_empty());
        assert_eq!(pos.summary.total_bank_cash_minor, 0);
        assert_eq!(pos.summary.total_cc_liability_minor, 0);
        assert_eq!(pos.summary.net_position_minor, 0);
        assert!(pos.summary.currencies.is_empty());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn cash_position_separates_bank_and_cc() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Create a bank account and a CC account
        let bank = account_svc::create_bank_account(
            &pool,
            TEST_APP,
            &CreateBankAccountRequest {
                account_name: "Checking".to_string(),
                institution: Some("First Bank".to_string()),
                account_number_last4: Some("1111".to_string()),
                routing_number: None,
                currency: "USD".to_string(),
                metadata: None,
            },
            None,
            "cp-test-1".to_string(),
        )
        .await
        .expect("create bank failed");

        let cc = account_svc::create_credit_card_account(
            &pool,
            TEST_APP,
            &CreateCreditCardAccountRequest {
                account_name: "Corp Visa".to_string(),
                institution: Some("Chase".to_string()),
                account_number_last4: Some("9999".to_string()),
                currency: "USD".to_string(),
                credit_limit_minor: Some(500_000),
                statement_closing_day: Some(15),
                cc_network: Some("Visa".to_string()),
                metadata: None,
            },
            None,
            "cp-test-2".to_string(),
        )
        .await
        .expect("create CC failed");

        // Insert transactions for both
        sqlx::query(
            r#"INSERT INTO treasury_bank_transactions
               (app_id, account_id, transaction_date, amount_minor, currency, external_id)
               VALUES ($1, $2, '2026-02-01', 100000, 'USD', 'cp-bank-1')"#,
        )
        .bind(TEST_APP)
        .bind(bank.id)
        .execute(&pool)
        .await
        .expect("insert bank txn failed");

        sqlx::query(
            r#"INSERT INTO treasury_bank_transactions
               (app_id, account_id, transaction_date, amount_minor, currency, external_id)
               VALUES ($1, $2, '2026-02-05', -30000, 'USD', 'cp-cc-1')"#,
        )
        .bind(TEST_APP)
        .bind(cc.id)
        .execute(&pool)
        .await
        .expect("insert cc txn failed");

        let pos = get_cash_position(&pool, TEST_APP)
            .await
            .expect("query failed");

        // Bank cash bucket
        assert_eq!(pos.bank_cash.len(), 1);
        assert_eq!(pos.bank_cash[0].account_id, bank.id);
        assert_eq!(pos.bank_cash[0].balance_minor, 100_000);

        // CC liability bucket
        assert_eq!(pos.credit_card_liability.len(), 1);
        assert_eq!(pos.credit_card_liability[0].account_id, cc.id);
        assert_eq!(pos.credit_card_liability[0].balance_minor, -30_000);

        // Summary
        assert_eq!(pos.summary.total_bank_cash_minor, 100_000);
        assert_eq!(pos.summary.total_cc_liability_minor, -30_000);
        assert_eq!(pos.summary.net_position_minor, 70_000);
        assert_eq!(pos.summary.currencies, vec!["USD"]);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn cash_position_includes_opening_balance_from_statement() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let bank = account_svc::create_bank_account(
            &pool,
            TEST_APP,
            &CreateBankAccountRequest {
                account_name: "Savings".to_string(),
                institution: None,
                account_number_last4: None,
                routing_number: None,
                currency: "EUR".to_string(),
                metadata: None,
            },
            None,
            "cp-test-3".to_string(),
        )
        .await
        .expect("create failed");

        // Insert a statement with opening balance
        sqlx::query(
            r#"INSERT INTO treasury_bank_statements
               (app_id, account_id, period_start, period_end,
                opening_balance_minor, closing_balance_minor, currency, status)
               VALUES ($1, $2, '2026-01-01', '2026-01-31', 500000, 520000, 'EUR',
                       'reconciled'::treasury_statement_status)"#,
        )
        .bind(TEST_APP)
        .bind(bank.id)
        .execute(&pool)
        .await
        .expect("insert statement failed");

        // Insert a transaction after the statement
        sqlx::query(
            r#"INSERT INTO treasury_bank_transactions
               (app_id, account_id, transaction_date, amount_minor, currency, external_id)
               VALUES ($1, $2, '2026-02-10', 15000, 'EUR', 'cp-eur-1')"#,
        )
        .bind(TEST_APP)
        .bind(bank.id)
        .execute(&pool)
        .await
        .expect("insert txn failed");

        let pos = get_cash_position(&pool, TEST_APP)
            .await
            .expect("query failed");

        assert_eq!(pos.bank_cash.len(), 1);
        // opening (500000) + txn (15000) = 515000
        assert_eq!(pos.bank_cash[0].opening_balance_minor, 500_000);
        assert_eq!(pos.bank_cash[0].transaction_total_minor, 15_000);
        assert_eq!(pos.bank_cash[0].balance_minor, 515_000);
        assert_eq!(pos.summary.currencies, vec!["EUR"]);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn cash_position_tenant_isolation() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Create account under TEST_APP
        account_svc::create_bank_account(
            &pool,
            TEST_APP,
            &CreateBankAccountRequest {
                account_name: "Isolated".to_string(),
                institution: None,
                account_number_last4: None,
                routing_number: None,
                currency: "USD".to_string(),
                metadata: None,
            },
            None,
            "cp-test-4".to_string(),
        )
        .await
        .expect("create failed");

        // Query different tenant — should see nothing
        let pos = get_cash_position(&pool, "other-tenant")
            .await
            .expect("query failed");

        assert!(pos.bank_cash.is_empty());
        assert!(pos.credit_card_liability.is_empty());
        assert_eq!(pos.summary.net_position_minor, 0);

        cleanup(&pool).await;
    }
}
