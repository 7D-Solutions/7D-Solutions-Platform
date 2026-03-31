//! Idempotency key handling for production POST endpoints.
//!
//! Before enqueueing to the outbox, callers check the production_idempotency_keys
//! table.  If the key exists with a matching request hash the cached response is
//! returned (replay).  If the hash differs a 409 Conflict is raised.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, Postgres, Transaction};

/// Row returned from the idempotency table lookup.
#[derive(Debug, FromRow)]
pub struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
    pub status_code: i16,
}

/// Error specific to idempotency checks.
#[derive(Debug, thiserror::Error)]
pub enum IdempotencyError {
    #[error("Conflicting idempotency key: same key was used with a different request body")]
    Conflict,
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Deserialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Look up an idempotency key.  Returns `Some(record)` if found, `None` otherwise.
pub async fn find_idempotency_key(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash, status_code
        FROM production_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

/// Check idempotency inside an open transaction.
///
/// Returns `Ok(Some(response_body_json))` on replay (hash matches).
/// Returns `Err(IdempotencyError::Conflict)` if the key exists but hash differs.
/// Returns `Ok(None)` if no prior key — caller should proceed normally.
pub async fn check_idempotency(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    request_hash: &str,
) -> Result<Option<String>, IdempotencyError> {
    if let Some(record) = find_idempotency_key(tx, tenant_id, idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(IdempotencyError::Conflict);
        }
        return Ok(Some(record.response_body));
    }
    Ok(None)
}

/// Store an idempotency key inside the caller's transaction.
pub async fn store_idempotency_key(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    request_hash: &str,
    response_json: &str,
    status_code: i16,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO production_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, $5, $6)
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(response_json)
    .bind(status_code)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
