//! Repository for numbering sequence and issued-number queries.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ── Row types ────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
pub struct IssuedRowFull {
    pub number_value: i64,
    pub status: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct IssuedRow {
    pub number_value: i64,
    pub status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SequenceRow {
    pub current_value: i64,
    pub gap_free: bool,
    pub reservation_ttl_secs: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct CounterRow {
    pub current_value: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct RecyclableRow {
    pub number_value: i64,
}

// ── Issued-number queries ────────────────────────────────────────────

pub async fn find_issued_by_idempotency_key(
    pool: &PgPool,
    tenant_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<IssuedRowFull>, sqlx::Error> {
    sqlx::query_as::<_, IssuedRowFull>(
        "SELECT number_value, status, expires_at \
         FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

pub async fn update_recycled_issued_tx(
    tx: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    status: &str,
    expires_at: Option<DateTime<Utc>>,
    tenant_id: Uuid,
    entity: &str,
    number_value: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE issued_numbers \
         SET idempotency_key = $1, status = $2, expires_at = $3 \
         WHERE tenant_id = $4 AND entity = $5 AND number_value = $6",
    )
    .bind(idempotency_key)
    .bind(status)
    .bind(expires_at)
    .bind(tenant_id)
    .bind(entity)
    .bind(number_value)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_issued_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    entity: &str,
    number_value: i64,
    idempotency_key: &str,
    status: &str,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO issued_numbers \
         (tenant_id, entity, number_value, idempotency_key, status, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tenant_id)
    .bind(entity)
    .bind(number_value)
    .bind(idempotency_key)
    .bind(status)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Confirm queries ──────────────────────────────────────────────────

pub async fn find_issued_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<IssuedRow>, sqlx::Error> {
    sqlx::query_as::<_, IssuedRow>(
        "SELECT number_value, status FROM issued_numbers \
         WHERE tenant_id = $1 AND idempotency_key = $2 \
         FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn confirm_issued_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    idempotency_key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE issued_numbers SET status = 'confirmed', expires_at = NULL \
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Sequence queries ─────────────────────────────────────────────────

pub async fn get_sequence_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    entity: &str,
) -> Result<Option<SequenceRow>, sqlx::Error> {
    sqlx::query_as::<_, SequenceRow>(
        "SELECT current_value, gap_free, reservation_ttl_secs \
         FROM sequences WHERE tenant_id = $1 AND entity = $2 FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn advance_counter_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    entity: &str,
) -> Result<CounterRow, sqlx::Error> {
    sqlx::query_as::<_, CounterRow>(
        "UPDATE sequences SET current_value = current_value + 1, updated_at = NOW() \
         WHERE tenant_id = $1 AND entity = $2 RETURNING current_value",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_one(&mut **tx)
    .await
}

pub async fn upsert_sequence_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    entity: &str,
    gap_free: bool,
) -> Result<SequenceRow, sqlx::Error> {
    sqlx::query_as::<_, SequenceRow>(
        "INSERT INTO sequences (tenant_id, entity, current_value, gap_free) \
         VALUES ($1, $2, 1, $3) \
         ON CONFLICT (tenant_id, entity) \
         DO UPDATE SET current_value = sequences.current_value + 1, updated_at = NOW() \
         RETURNING current_value, gap_free, reservation_ttl_secs",
    )
    .bind(tenant_id)
    .bind(entity)
    .bind(gap_free)
    .fetch_one(&mut **tx)
    .await
}

pub async fn find_recyclable_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    entity: &str,
) -> Result<Option<RecyclableRow>, sqlx::Error> {
    sqlx::query_as::<_, RecyclableRow>(
        "SELECT number_value FROM issued_numbers \
         WHERE tenant_id = $1 AND entity = $2 \
           AND status = 'reserved' AND expires_at < NOW() \
         ORDER BY number_value ASC LIMIT 1 \
         FOR UPDATE SKIP LOCKED",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_optional(&mut **tx)
    .await
}
