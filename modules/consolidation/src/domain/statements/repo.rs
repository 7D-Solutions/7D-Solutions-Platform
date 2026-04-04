//! Repository layer — all SQL access for consolidated statements.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

/// Fetch TB cache rows for a group+as_of. Used by both BS and P&L.
pub async fn fetch_tb_cache_rows(
    pool: &PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
) -> Result<Vec<(String, String, String, i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT account_code,
               account_name,
               currency,
               debit_minor,
               credit_minor
        FROM csl_trial_balance_cache
        WHERE group_id = $1
          AND as_of = $2
        ORDER BY account_code, currency
        "#,
    )
    .bind(group_id)
    .bind(as_of)
    .fetch_all(pool)
    .await
}
