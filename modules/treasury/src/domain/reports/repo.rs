//! Repository layer — all SQL access for the reports domain.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::accounts::AccountType;

// ============================================================================
// Row types (internal; pub so sibling modules can iterate fields)
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AccountPositionRow {
    pub account_id: Uuid,
    pub account_name: String,
    pub account_type: AccountType,
    pub currency: String,
    pub institution: Option<String>,
    pub opening_balance_minor: i64,
    pub transaction_total_minor: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ArAgingRow {
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub days_over_90_minor: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ApAgingRow {
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub over_90_minor: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SchedRow {
    pub currency: String,
    pub total_minor: i64,
}

// ============================================================================
// Cash position query
// ============================================================================

pub async fn fetch_account_positions(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<AccountPositionRow>, sqlx::Error> {
    sqlx::query_as::<_, AccountPositionRow>(
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
    .await
}

// ============================================================================
// Forecast queries (cross-module reads — pool may be from AR or AP database)
// ============================================================================

pub async fn fetch_ar_aging(
    ar_pool: &PgPool,
    app_id: &str,
) -> Result<Vec<ArAgingRow>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT
            currency,
            COALESCE(SUM(current_minor), 0)::bigint       AS current_minor,
            COALESCE(SUM(days_1_30_minor), 0)::bigint      AS days_1_30_minor,
            COALESCE(SUM(days_31_60_minor), 0)::bigint     AS days_31_60_minor,
            COALESCE(SUM(days_61_90_minor), 0)::bigint     AS days_61_90_minor,
            COALESCE(SUM(days_over_90_minor), 0)::bigint   AS days_over_90_minor
        FROM ar_aging_buckets
        WHERE app_id = $1
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(app_id)
    .fetch_all(ar_pool)
    .await
}

pub async fn fetch_ap_aging(
    ap_pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<ApAgingRow>, sqlx::Error> {
    sqlx::query_as(
        r#"
        WITH bill_open AS (
            SELECT
                b.currency,
                b.due_date,
                (b.total_minor - COALESCE(SUM(a.amount_minor), 0)) AS open_minor
            FROM vendor_bills b
            LEFT JOIN ap_allocations a
                ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
            WHERE b.tenant_id = $1
              AND b.status IN ('approved', 'partially_paid')
            GROUP BY b.bill_id, b.currency, b.due_date, b.total_minor
            HAVING (b.total_minor - COALESCE(SUM(a.amount_minor), 0)) > 0
        )
        SELECT
            currency,
            COALESCE(SUM(CASE WHEN due_date >= NOW()
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS current_minor,
            COALESCE(SUM(CASE WHEN due_date >= NOW() - INTERVAL '30 days'
                               AND due_date < NOW()
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN due_date >= NOW() - INTERVAL '60 days'
                               AND due_date < NOW() - INTERVAL '30 days'
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN due_date >= NOW() - INTERVAL '90 days'
                               AND due_date < NOW() - INTERVAL '60 days'
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN due_date < NOW() - INTERVAL '90 days'
                              THEN open_minor ELSE 0 END), 0)::bigint
                AS over_90_minor
        FROM bill_open
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .fetch_all(ap_pool)
    .await
}

pub async fn fetch_scheduled_payments(
    ap_pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<SchedRow>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT
            currency,
            COALESCE(SUM(total_minor), 0)::bigint AS total_minor
        FROM payment_runs
        WHERE tenant_id = $1
          AND status = 'pending'
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .fetch_all(ap_pool)
    .await
}

// ============================================================================
// Test helpers
// ============================================================================

#[cfg(test)]
pub async fn delete_test_cash_position_data(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM treasury_recon_matches WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_statements WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'bank_account' AND aggregate_id IN \
         (SELECT id::TEXT FROM treasury_bank_accounts WHERE app_id = $1)",
    )
    .bind(app_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

#[cfg(test)]
pub async fn insert_test_bank_txn(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    transaction_date: &str,
    amount_minor: i64,
    currency: &str,
    external_id: &str,
) {
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, $3::date, $4, $5, $6)"#,
    )
    .bind(app_id)
    .bind(account_id)
    .bind(transaction_date)
    .bind(amount_minor)
    .bind(currency)
    .bind(external_id)
    .execute(pool)
    .await
    .expect("insert_test_bank_txn failed");
}

#[cfg(test)]
pub async fn insert_test_statement(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    period_start: &str,
    period_end: &str,
    opening_balance_minor: i64,
    closing_balance_minor: i64,
    currency: &str,
    status: &str,
) {
    sqlx::query(
        r#"INSERT INTO treasury_bank_statements
           (app_id, account_id, period_start, period_end,
            opening_balance_minor, closing_balance_minor, currency, status)
           VALUES ($1, $2, $3::date, $4::date, $5, $6, $7, $8::treasury_statement_status)"#,
    )
    .bind(app_id)
    .bind(account_id)
    .bind(period_start)
    .bind(period_end)
    .bind(opening_balance_minor)
    .bind(closing_balance_minor)
    .bind(currency)
    .bind(status)
    .execute(pool)
    .await
    .expect("insert_test_statement failed");
}
