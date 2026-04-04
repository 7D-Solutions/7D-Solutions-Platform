//! Repository layer for EDI transaction persistence.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::EdiTransaction;

pub async fn get_by_id(
    pool: &PgPool,
    tenant_id: &str,
    transaction_id: Uuid,
) -> Result<Option<EdiTransaction>, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"SELECT id, tenant_id, transaction_type, version, direction,
                  raw_payload, parsed_payload, validation_status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_edi_transactions
           WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(transaction_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_by_tenant(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<EdiTransaction>, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"SELECT id, tenant_id, transaction_type, version, direction,
                  raw_payload, parsed_payload, validation_status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_edi_transactions
           WHERE tenant_id = $1
           ORDER BY created_at DESC"#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_idempotency_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<EdiTransaction>, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"SELECT id, tenant_id, transaction_type, version, direction,
                  raw_payload, parsed_payload, validation_status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_edi_transactions
           WHERE tenant_id = $1 AND idempotency_key = $2"#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn insert_inbound(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    tenant_id: &str,
    transaction_type: &str,
    version: &str,
    direction: &str,
    raw_payload: &str,
    status: &str,
    idempotency_key: &Option<String>,
) -> Result<EdiTransaction, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"INSERT INTO integrations_edi_transactions
               (id, tenant_id, transaction_type, version, direction,
                raw_payload, validation_status, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, tenant_id, transaction_type, version, direction,
                     raw_payload, parsed_payload, validation_status,
                     error_details, idempotency_key, created_at, updated_at"#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(transaction_type)
    .bind(version)
    .bind(direction)
    .bind(raw_payload)
    .bind(status)
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await
}

pub async fn insert_outbound(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    tenant_id: &str,
    transaction_type: &str,
    version: &str,
    direction: &str,
    parsed_payload: &serde_json::Value,
    status: &str,
    idempotency_key: &Option<String>,
) -> Result<EdiTransaction, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"INSERT INTO integrations_edi_transactions
               (id, tenant_id, transaction_type, version, direction,
                parsed_payload, validation_status, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, tenant_id, transaction_type, version, direction,
                     raw_payload, parsed_payload, validation_status,
                     error_details, idempotency_key, created_at, updated_at"#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(transaction_type)
    .bind(version)
    .bind(direction)
    .bind(parsed_payload)
    .bind(status)
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    transaction_id: Uuid,
    tenant_id: &str,
) -> Result<Option<EdiTransaction>, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"SELECT id, tenant_id, transaction_type, version, direction,
                  raw_payload, parsed_payload, validation_status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_edi_transactions
           WHERE id = $1 AND tenant_id = $2
           FOR UPDATE"#,
    )
    .bind(transaction_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn update_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    new_status: &str,
    error_details: &Option<String>,
    parsed_payload: &Option<serde_json::Value>,
    transaction_id: Uuid,
    tenant_id: &str,
) -> Result<EdiTransaction, sqlx::Error> {
    sqlx::query_as::<_, EdiTransaction>(
        r#"UPDATE integrations_edi_transactions
           SET validation_status = $1,
               error_details = $2,
               parsed_payload = COALESCE($3, parsed_payload),
               updated_at = NOW()
           WHERE id = $4 AND tenant_id = $5
           RETURNING id, tenant_id, transaction_type, version, direction,
                     raw_payload, parsed_payload, validation_status,
                     error_details, idempotency_key, created_at, updated_at"#,
    )
    .bind(new_status)
    .bind(error_details)
    .bind(parsed_payload)
    .bind(transaction_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await
}
