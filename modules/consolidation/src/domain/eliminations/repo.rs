//! Repository layer — all SQL access for elimination postings.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn fetch_existing_posting(
    pool: &PgPool,
    group_id: Uuid,
    period_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<(DateTime<Utc>, serde_json::Value, chrono::NaiveDate)>, sqlx::Error> {
    sqlx::query_as::<_, (DateTime<Utc>, serde_json::Value, chrono::NaiveDate)>(
        "SELECT posted_at, journal_entry_ids, (posted_at::date) as as_of_date
         FROM csl_elimination_postings
         WHERE group_id = $1 AND period_id = $2 AND idempotency_key = $3",
    )
    .bind(group_id)
    .bind(period_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

pub async fn insert_posting(
    pool: &PgPool,
    group_id: Uuid,
    period_id: Uuid,
    idempotency_key: &str,
    journal_entry_ids: &serde_json::Value,
    suggestion_count: i32,
    total_amount_minor: i64,
    posted_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO csl_elimination_postings
            (group_id, period_id, idempotency_key, journal_entry_ids,
             suggestion_count, total_amount_minor, posted_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (group_id, period_id, idempotency_key) DO NOTHING",
    )
    .bind(group_id)
    .bind(period_id)
    .bind(idempotency_key)
    .bind(journal_entry_ids)
    .bind(suggestion_count)
    .bind(total_amount_minor)
    .bind(posted_at)
    .execute(pool)
    .await?;
    Ok(())
}
