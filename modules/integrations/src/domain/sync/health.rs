use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SyncJobRow {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub job_name: String,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub failure_streak: i32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const SELECT_COLS: &str = r#"
    id, app_id, provider, job_name,
    last_success_at, last_failure_at,
    failure_streak, last_error,
    created_at, updated_at
"#;

/// Record a successful tick for `(app_id, provider, job_name)`.
/// Resets failure_streak to 0 and clears last_error.
pub async fn upsert_job_success(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    job_name: &str,
) -> Result<SyncJobRow, sqlx::Error> {
    sqlx::query_as::<_, SyncJobRow>(&format!(
        r#"
        INSERT INTO integrations_sync_jobs
            (app_id, provider, job_name, last_success_at, failure_streak, updated_at)
        VALUES ($1, $2, $3, NOW(), 0, NOW())
        ON CONFLICT (app_id, provider, job_name) DO UPDATE
            SET last_success_at = NOW(),
                failure_streak  = 0,
                last_error      = NULL,
                updated_at      = NOW()
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(job_name)
    .fetch_one(pool)
    .await
}

/// Record a failed tick for `(app_id, provider, job_name)`.
/// Increments failure_streak and records the error message.
pub async fn upsert_job_failure(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    job_name: &str,
    error: &str,
) -> Result<SyncJobRow, sqlx::Error> {
    sqlx::query_as::<_, SyncJobRow>(&format!(
        r#"
        INSERT INTO integrations_sync_jobs
            (app_id, provider, job_name, last_failure_at, failure_streak, last_error, updated_at)
        VALUES ($1, $2, $3, NOW(), 1, $4, NOW())
        ON CONFLICT (app_id, provider, job_name) DO UPDATE
            SET last_failure_at = NOW(),
                failure_streak  = integrations_sync_jobs.failure_streak + 1,
                last_error      = $4,
                updated_at      = NOW()
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(job_name)
    .bind(error)
    .fetch_one(pool)
    .await
}

/// List all sync job health rows for a tenant, ordered by provider + job_name.
/// Returns `(rows, total_count)`.
pub async fn list_jobs(
    pool: &PgPool,
    app_id: &str,
    page: i64,
    page_size: i64,
) -> Result<(Vec<SyncJobRow>, i64), sqlx::Error> {
    let offset = (page - 1).max(0) * page_size;

    let rows = sqlx::query_as::<_, SyncJobRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_jobs
        WHERE app_id = $1
        ORDER BY provider, job_name
        LIMIT $2 OFFSET $3
        "#
    ))
    .bind(app_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_jobs WHERE app_id = $1")
            .bind(app_id)
            .fetch_one(pool)
            .await?;

    Ok((rows, total.0))
}
