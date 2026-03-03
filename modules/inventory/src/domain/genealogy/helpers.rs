//! Shared database helpers for genealogy operations.

use chrono::DateTime;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(super) struct LotRow {
    pub id: Uuid,
    pub lot_code: String,
}

#[derive(sqlx::FromRow)]
pub(super) struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
}

pub(super) async fn find_lot(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    lot_code: &str,
) -> Result<Option<LotRow>, sqlx::Error> {
    sqlx::query_as::<_, LotRow>(
        r#"
        SELECT id, lot_code
        FROM inventory_lots
        WHERE tenant_id = $1 AND item_id = $2 AND lot_code = $3
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(lot_code)
    .fetch_optional(pool)
    .await
}

pub(super) async fn upsert_lot_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    lot_code: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_lots (tenant_id, item_id, lot_code)
        VALUES ($1, $2, $3)
        ON CONFLICT (tenant_id, item_id, lot_code) DO UPDATE
            SET tenant_id = EXCLUDED.tenant_id
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(lot_code)
    .fetch_one(&mut **tx)
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_genealogy_edge(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    operation_id: Uuid,
    parent_lot_id: Uuid,
    child_lot_id: Uuid,
    transformation: &str,
    quantity: i64,
    occurred_at: DateTime<chrono::Utc>,
    actor_id: Option<Uuid>,
    notes: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inv_lot_genealogy
            (tenant_id, operation_id, parent_lot_id, child_lot_id, transformation,
             quantity, occurred_at, actor_id, notes)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(operation_id)
    .bind(parent_lot_id)
    .bind(child_lot_id)
    .bind(transformation)
    .bind(quantity)
    .bind(occurred_at)
    .bind(actor_id)
    .bind(notes)
    .fetch_one(&mut **tx)
    .await
}

/// Sum of quantity_remaining across all FIFO layers for a given lot.
pub(super) async fn lot_on_hand(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COALESCE(SUM(quantity_remaining), 0)::BIGINT
        FROM inventory_layers
        WHERE tenant_id = $1 AND lot_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .fetch_one(pool)
    .await
}

pub(super) async fn find_idempotency_key(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash
        FROM inv_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}
