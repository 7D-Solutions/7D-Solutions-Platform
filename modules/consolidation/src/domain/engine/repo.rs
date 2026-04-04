//! Repository layer — all SQL access for the consolidation engine.

use chrono::NaiveDate;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::compute::CachedTbRow;
use super::ConsolidatedTbRow;

pub async fn delete_cache_rows(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    group_id: Uuid,
    as_of: NaiveDate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM csl_trial_balance_cache WHERE group_id = $1 AND as_of = $2 \
         AND group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $3)",
    )
    .bind(group_id)
    .bind(as_of)
    .bind(tenant_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_cache_row(
    tx: &mut Transaction<'_, Postgres>,
    group_id: Uuid,
    as_of: NaiveDate,
    row: &ConsolidatedTbRow,
    currency: &str,
    input_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO csl_trial_balance_cache
            (group_id, as_of, account_code, account_name, currency, debit_minor, credit_minor, net_minor, input_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(group_id)
    .bind(as_of)
    .bind(&row.account_code)
    .bind(&row.account_name)
    .bind(currency)
    .bind(row.debit_minor)
    .bind(row.credit_minor)
    .bind(row.net_minor)
    .bind(input_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn fetch_cached_tb(
    pool: &PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
) -> Result<Vec<CachedTbRow>, sqlx::Error> {
    sqlx::query_as::<_, CachedTbRow>(
        "SELECT account_code, account_name, currency, debit_minor, credit_minor, net_minor, input_hash, computed_at
         FROM csl_trial_balance_cache
         WHERE group_id = $1 AND as_of = $2
         ORDER BY account_code",
    )
    .bind(group_id)
    .bind(as_of)
    .fetch_all(pool)
    .await
}
