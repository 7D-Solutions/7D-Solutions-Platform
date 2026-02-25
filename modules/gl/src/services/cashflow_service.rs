//! Cash Flow Statement Service (Phase 24b, bd-2w3)
//!
//! Derives cash flow by period from GL journal lines, classified into
//! operating / investing / financing categories via the cashflow_classifications table.
//!
//! **Key invariant:** Cash flow totals must reconcile to the net change in
//! cash accounts for the period (sum of debits − credits on cash accounts).
//!
//! **Derivation:** Scans journal_lines for classified accounts within the period,
//! aggregates net amounts per account, groups by category.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::statements::{CashFlowCategoryTotal, CashFlowRow};

/// Cash flow response with rows, category totals, and reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashFlowResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String,
    pub rows: Vec<CashFlowRow>,
    pub category_totals: Vec<CashFlowCategoryTotal>,
    /// Sum of all categories (should equal net change in cash accounts)
    pub net_cash_flow: i64,
    /// Net change in designated cash accounts (for reconciliation)
    pub cash_account_net_change: i64,
    /// Whether net_cash_flow == cash_account_net_change
    pub reconciles: bool,
}

#[derive(Debug, Error)]
pub enum CashFlowError {
    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Invalid currency: {0}")]
    InvalidCurrency(String),

    #[error("Period not found: period_id={period_id}, tenant_id={tenant_id}")]
    PeriodNotFound { period_id: Uuid, tenant_id: String },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Internal DB row for cash flow query result.
#[derive(Debug, Clone, FromRow)]
struct CashFlowRowDb {
    pub account_code: String,
    pub account_name: String,
    pub category: String,
    pub currency: String,
    /// Net amount = SUM(debit_minor) - SUM(credit_minor) on journal lines
    pub net_amount: i64,
}

/// Internal DB row for cash account net change.
#[derive(Debug, Clone, FromRow)]
struct CashAccountDelta {
    pub net_change: i64,
}

/// Get cash flow statement for a tenant and period.
///
/// Derives cash flow from journal lines for accounts that have a
/// cashflow_classifications entry. Groups by category and reconciles
/// against cash account (asset-type) balance deltas.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Currency code (ISO 4217, required)
/// * `cash_account_codes` - Account codes designated as "cash" for reconciliation
///
/// # Returns
/// Cash flow response with rows, category totals, and reconciliation check
pub async fn get_cash_flow(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,
    cash_account_codes: &[String],
) -> Result<CashFlowResponse, CashFlowError> {
    if tenant_id.is_empty() {
        return Err(CashFlowError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }

    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(CashFlowError::InvalidCurrency(currency.to_string()));
    }

    // Verify period exists and belongs to tenant
    let period_exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM accounting_periods WHERE id = $1 AND tenant_id = $2",
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    if period_exists.is_none() {
        return Err(CashFlowError::PeriodNotFound {
            period_id,
            tenant_id: tenant_id.to_string(),
        });
    }

    // Query cash flow rows: aggregate journal lines by classified accounts within the period.
    // JOIN journal_entries (for period/tenant/currency filter) →
    //   journal_lines (for debit/credit amounts) →
    //   cashflow_classifications (for category mapping) →
    //   accounts (for account name)
    let db_rows: Vec<CashFlowRowDb> = sqlx::query_as(
        r#"
        SELECT
            jl.account_ref AS account_code,
            a.name AS account_name,
            cc.category::TEXT AS category,
            je.currency,
            COALESCE(SUM(jl.debit_minor) - SUM(jl.credit_minor), 0)::BIGINT AS net_amount
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        INNER JOIN cashflow_classifications cc
            ON cc.tenant_id = je.tenant_id AND cc.account_code = jl.account_ref
        INNER JOIN accounts a
            ON a.tenant_id = je.tenant_id AND a.code = jl.account_ref
        INNER JOIN accounting_periods p
            ON p.id = $3 AND p.tenant_id = je.tenant_id
        WHERE je.tenant_id = $1
          AND je.currency = $2
          AND je.posted_at >= p.period_start::timestamptz
          AND je.posted_at < (p.period_end + INTERVAL '1 day')::timestamptz
        GROUP BY jl.account_ref, a.name, cc.category, je.currency
        ORDER BY cc.category, jl.account_ref
        "#,
    )
    .bind(tenant_id)
    .bind(currency)
    .bind(period_id)
    .fetch_all(pool)
    .await?;

    // Convert to domain rows
    let rows: Vec<CashFlowRow> = db_rows
        .iter()
        .map(|r| CashFlowRow {
            account_code: r.account_code.clone(),
            account_name: r.account_name.clone(),
            category: r.category.clone(),
            currency: r.currency.clone(),
            amount_minor: r.net_amount,
        })
        .collect();

    // Compute category totals
    let mut operating_total: i64 = 0;
    let mut investing_total: i64 = 0;
    let mut financing_total: i64 = 0;

    for row in &db_rows {
        match row.category.as_str() {
            "operating" => operating_total += row.net_amount,
            "investing" => investing_total += row.net_amount,
            "financing" => financing_total += row.net_amount,
            _ => {} // ignore unexpected categories
        }
    }

    let category_totals = vec![
        CashFlowCategoryTotal {
            category: "operating".to_string(),
            total_minor: operating_total,
        },
        CashFlowCategoryTotal {
            category: "investing".to_string(),
            total_minor: investing_total,
        },
        CashFlowCategoryTotal {
            category: "financing".to_string(),
            total_minor: financing_total,
        },
    ];

    let net_cash_flow = operating_total + investing_total + financing_total;

    // Compute cash account net change for reconciliation.
    // This is the actual change in cash accounts (from account_balances).
    let cash_account_net_change = if cash_account_codes.is_empty() {
        0
    } else {
        let delta: Option<CashAccountDelta> = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(net_balance_minor), 0)::BIGINT AS net_change
            FROM account_balances
            WHERE tenant_id = $1
              AND period_id = $2
              AND currency = $3
              AND account_code = ANY($4)
            "#,
        )
        .bind(tenant_id)
        .bind(period_id)
        .bind(currency)
        .bind(cash_account_codes)
        .fetch_optional(pool)
        .await?;

        delta.map(|d| d.net_change).unwrap_or(0)
    };

    let reconciles = net_cash_flow == cash_account_net_change;

    Ok(CashFlowResponse {
        tenant_id: tenant_id.to_string(),
        period_id,
        currency: currency.to_string(),
        rows,
        category_totals,
        net_cash_flow,
        cash_account_net_change,
        reconciles,
    })
}
