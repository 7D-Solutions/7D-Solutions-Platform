//! Repository layer for QBO CDC/sync connection queries.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct CdcConnection {
    pub id: Uuid,
    pub app_id: String,
    pub realm_id: String,
    pub cdc_watermark: Option<DateTime<Utc>>,
    pub full_resync_required: bool,
}

/// Fetch all connected QBO OAuth connections for CDC polling.
pub async fn get_connected_qbo_connections(
    pool: &PgPool,
) -> Result<Vec<CdcConnection>, sqlx::Error> {
    sqlx::query_as::<_, CdcConnection>(
        r#"
        SELECT id, app_id, realm_id, cdc_watermark, full_resync_required
        FROM integrations_oauth_connections
        WHERE connection_status = 'connected'
          AND provider = 'quickbooks'
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Flag a connection as requiring full resync.
pub async fn set_full_resync_required(
    pool: &PgPool,
    connection_id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "UPDATE integrations_oauth_connections \
         SET full_resync_required = TRUE, updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(connection_id)
    .execute(pool)
    .await
}

/// Advance the CDC watermark after a successful poll.
pub async fn advance_cdc_watermark(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    now: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "UPDATE integrations_oauth_connections \
         SET cdc_watermark = $1, updated_at = $1 \
         WHERE app_id = $2 AND provider = 'quickbooks'",
    )
    .bind(now)
    .bind(app_id)
    .execute(&mut **tx)
    .await
}

/// Clear the full_resync flag and set cdc_watermark after a completed resync.
pub async fn mark_resync_complete(
    pool: &PgPool,
    app_id: &str,
    now: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "UPDATE integrations_oauth_connections \
         SET full_resync_required = FALSE, cdc_watermark = $1, updated_at = $1 \
         WHERE app_id = $2 AND provider = 'quickbooks'",
    )
    .bind(now)
    .bind(app_id)
    .execute(pool)
    .await
}
