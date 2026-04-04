//! Repository layer for file job persistence.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::FileJob;

pub async fn get_by_id(
    pool: &PgPool,
    tenant_id: &str,
    job_id: Uuid,
) -> Result<Option<FileJob>, sqlx::Error> {
    sqlx::query_as::<_, FileJob>(
        r#"SELECT id, tenant_id, file_ref, parser_type, status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_file_jobs
           WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(job_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_by_tenant(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<FileJob>, sqlx::Error> {
    sqlx::query_as::<_, FileJob>(
        r#"SELECT id, tenant_id, file_ref, parser_type, status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_file_jobs
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
) -> Result<Option<FileJob>, sqlx::Error> {
    sqlx::query_as::<_, FileJob>(
        r#"SELECT id, tenant_id, file_ref, parser_type, status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_file_jobs
           WHERE tenant_id = $1 AND idempotency_key = $2"#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    tenant_id: &str,
    file_ref: &str,
    parser_type: &str,
    status: &str,
    idempotency_key: &Option<String>,
) -> Result<FileJob, sqlx::Error> {
    sqlx::query_as::<_, FileJob>(
        r#"INSERT INTO integrations_file_jobs
               (id, tenant_id, file_ref, parser_type, status, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, tenant_id, file_ref, parser_type, status,
                     error_details, idempotency_key, created_at, updated_at"#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(file_ref)
    .bind(parser_type)
    .bind(status)
    .bind(idempotency_key)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: Uuid,
    tenant_id: &str,
) -> Result<Option<FileJob>, sqlx::Error> {
    sqlx::query_as::<_, FileJob>(
        r#"SELECT id, tenant_id, file_ref, parser_type, status,
                  error_details, idempotency_key, created_at, updated_at
           FROM integrations_file_jobs
           WHERE id = $1 AND tenant_id = $2
           FOR UPDATE"#,
    )
    .bind(job_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn update_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    new_status: &str,
    error_details: &Option<String>,
    job_id: Uuid,
    tenant_id: &str,
) -> Result<FileJob, sqlx::Error> {
    sqlx::query_as::<_, FileJob>(
        r#"UPDATE integrations_file_jobs
           SET status = $1, error_details = $2, updated_at = NOW()
           WHERE id = $3 AND tenant_id = $4
           RETURNING id, tenant_id, file_ref, parser_type, status,
                     error_details, idempotency_key, created_at, updated_at"#,
    )
    .bind(new_status)
    .bind(error_details)
    .bind(job_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await
}
