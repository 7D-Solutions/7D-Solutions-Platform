use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::dedupe::truncate_to_millis;

// ── Domain model ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ObservationRow {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    pub fingerprint: String,
    pub last_updated_time: DateTime<Utc>,
    pub comparable_hash: String,
    pub projection_version: i32,
    pub raw_payload: Value,
    pub source_channel: String,
    pub is_tombstone: bool,
    pub observed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Repository ────────────────────────────────────────────────────────────────

const SELECT_COLS: &str = r#"
    id, app_id, provider, entity_type, entity_id, fingerprint,
    last_updated_time, comparable_hash, projection_version, raw_payload,
    source_channel, is_tombstone, observed_at, created_at, updated_at
"#;

/// Record an observation, deduplicating on (app_id, provider, entity_type, entity_id, fingerprint).
///
/// On conflict the row is updated if `projection_version`, `comparable_hash`, `source_channel`,
/// or `is_tombstone` changed.  `observed_at` is always refreshed so callers can distinguish
/// re-observations from the first time a fingerprint was seen.
///
/// `last_updated_time` MUST already be millisecond-truncated by the caller via
/// `dedupe::truncate_to_millis`.  The DB CHECK constraint will reject non-truncated
/// timestamps with a clear error.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_observation(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    entity_id: &str,
    fingerprint: &str,
    last_updated_time: DateTime<Utc>,
    comparable_hash: &str,
    projection_version: i32,
    raw_payload: &Value,
    source_channel: &str,
    is_tombstone: bool,
) -> Result<ObservationRow, sqlx::Error> {
    // Belt-and-suspenders: enforce truncation before hitting the DB check constraint.
    let lut = truncate_to_millis(last_updated_time);

    sqlx::query_as::<_, ObservationRow>(&format!(
        r#"
        INSERT INTO integrations_sync_observations
            (app_id, provider, entity_type, entity_id, fingerprint,
             last_updated_time, comparable_hash, projection_version, raw_payload,
             source_channel, is_tombstone)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT ON CONSTRAINT integrations_sync_observations_unique DO UPDATE
            SET comparable_hash    = EXCLUDED.comparable_hash,
                projection_version = EXCLUDED.projection_version,
                raw_payload        = EXCLUDED.raw_payload,
                source_channel     = EXCLUDED.source_channel,
                is_tombstone       = EXCLUDED.is_tombstone,
                observed_at        = NOW(),
                updated_at         = NOW()
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(entity_id)
    .bind(fingerprint)
    .bind(lut)
    .bind(comparable_hash)
    .bind(projection_version)
    .bind(raw_payload)
    .bind(source_channel)
    .bind(is_tombstone)
    .fetch_one(pool)
    .await
}

/// Fetch a single observation by primary key.
pub async fn get_observation(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<ObservationRow>, sqlx::Error> {
    sqlx::query_as::<_, ObservationRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_observations
        WHERE id = $1
        "#
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Fetch the latest observation for a specific entity (highest last_updated_time).
///
/// Used by CDC and webhook ingestion flows to retrieve the high-watermark
/// observation before deciding whether an incoming event advances the state.
pub async fn get_latest_for_entity(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    entity_id: &str,
) -> Result<Option<ObservationRow>, sqlx::Error> {
    sqlx::query_as::<_, ObservationRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_observations
        WHERE app_id = $1 AND provider = $2
          AND entity_type = $3 AND entity_id = $4
        ORDER BY last_updated_time DESC, observed_at DESC
        LIMIT 1
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(entity_id)
    .fetch_optional(pool)
    .await
}

/// List observations for a provider since a high-watermark timestamp (inclusive).
///
/// Returns rows ordered by `last_updated_time ASC` so callers can process them
/// in chronological order and advance their watermark incrementally.
pub async fn list_since_watermark(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    since: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ObservationRow>, sqlx::Error> {
    let since_ms = truncate_to_millis(since);
    sqlx::query_as::<_, ObservationRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_observations
        WHERE app_id = $1 AND provider = $2
          AND entity_type = $3
          AND last_updated_time >= $4
        ORDER BY last_updated_time ASC, id ASC
        LIMIT $5
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(since_ms)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Look up observations by comparable_hash.
///
/// A comparable_hash lookup is used by the correlation layer to check whether
/// a platform entity already matches a provider observation before dispatching
/// a sync write.
pub async fn find_by_comparable_hash(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    comparable_hash: &str,
) -> Result<Vec<ObservationRow>, sqlx::Error> {
    sqlx::query_as::<_, ObservationRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_observations
        WHERE app_id = $1 AND provider = $2
          AND entity_type = $3 AND comparable_hash = $4
        ORDER BY last_updated_time DESC
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(comparable_hash)
    .fetch_all(pool)
    .await
}

/// Paginated list of observations for a tenant / provider.
///
/// Returns `(rows, total_count)`.
pub async fn list_observations(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: Option<&str>,
    page: i64,
    page_size: i64,
) -> Result<(Vec<ObservationRow>, i64), sqlx::Error> {
    let offset = (page - 1).max(0) * page_size;

    let (data_sql, count_sql) = if let Some(_et) = entity_type {
        (
            format!(
                "SELECT {SELECT_COLS} FROM integrations_sync_observations \
                 WHERE app_id = $1 AND provider = $2 AND entity_type = $3 \
                 ORDER BY last_updated_time DESC LIMIT $4 OFFSET $5"
            ),
            "SELECT COUNT(*) FROM integrations_sync_observations \
             WHERE app_id = $1 AND provider = $2 AND entity_type = $3"
                .to_string(),
        )
    } else {
        (
            format!(
                "SELECT {SELECT_COLS} FROM integrations_sync_observations \
                 WHERE app_id = $1 AND provider = $2 \
                 ORDER BY last_updated_time DESC LIMIT $3 OFFSET $4"
            ),
            "SELECT COUNT(*) FROM integrations_sync_observations \
             WHERE app_id = $1 AND provider = $2"
                .to_string(),
        )
    };

    if let Some(et) = entity_type {
        let rows = sqlx::query_as::<_, ObservationRow>(&data_sql)
            .bind(app_id)
            .bind(provider)
            .bind(et)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let total: (i64,) = sqlx::query_as(&count_sql)
            .bind(app_id)
            .bind(provider)
            .bind(et)
            .fetch_one(pool)
            .await?;

        Ok((rows, total.0))
    } else {
        let rows = sqlx::query_as::<_, ObservationRow>(&data_sql)
            .bind(app_id)
            .bind(provider)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        let total: (i64,) = sqlx::query_as(&count_sql)
            .bind(app_id)
            .bind(provider)
            .fetch_one(pool)
            .await?;

        Ok((rows, total.0))
    }
}
