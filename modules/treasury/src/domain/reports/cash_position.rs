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

use crate::domain::accounts::AccountType;

use super::repo;

// ============================================================================
// Types
// ============================================================================

/// Position for a single account.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AccountPosition {
    pub account_id: uuid::Uuid,
    pub account_name: String,
    pub currency: String,
    pub institution: Option<String>,
    pub opening_balance_minor: i64,
    pub transaction_total_minor: i64,
    pub balance_minor: i64,
}

/// Aggregated summary across all accounts.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CashPositionSummary {
    pub total_bank_cash_minor: i64,
    pub total_cc_liability_minor: i64,
    pub net_position_minor: i64,
    pub currencies: Vec<String>,
}

/// Full cash position response.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
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
    let rows = repo::fetch_account_positions(pool, app_id).await?;

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
    use crate::domain::reports::repo as reports_repo;
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
        reports_repo::delete_test_cash_position_data(pool, TEST_APP).await;
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
        reports_repo::insert_test_bank_txn(
            &pool,
            TEST_APP,
            bank.id,
            "2026-02-01",
            100000,
            "USD",
            "cp-bank-1",
        )
        .await;
        reports_repo::insert_test_bank_txn(
            &pool,
            TEST_APP,
            cc.id,
            "2026-02-05",
            -30000,
            "USD",
            "cp-cc-1",
        )
        .await;

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
        reports_repo::insert_test_statement(
            &pool,
            TEST_APP,
            bank.id,
            "2026-01-01",
            "2026-01-31",
            500000,
            520000,
            "EUR",
            "reconciled",
        )
        .await;

        // Insert a transaction after the statement
        reports_repo::insert_test_bank_txn(
            &pool,
            TEST_APP,
            bank.id,
            "2026-02-10",
            15000,
            "EUR",
            "cp-eur-1",
        )
        .await;

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
