//! Repository layer for connector config persistence.

use sqlx::PgPool;
use uuid::Uuid;

use super::ConnectorConfig;

pub async fn get_config(
    pool: &PgPool,
    app_id: &str,
    id: Uuid,
) -> Result<Option<ConnectorConfig>, sqlx::Error> {
    sqlx::query_as::<_, ConnectorConfig>(
        r#"
        SELECT id, app_id, connector_type, name, config, enabled, created_at, updated_at
        FROM integrations_connector_configs
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_configs(
    pool: &PgPool,
    app_id: &str,
    enabled_only: bool,
) -> Result<Vec<ConnectorConfig>, sqlx::Error> {
    if enabled_only {
        sqlx::query_as::<_, ConnectorConfig>(
            r#"
            SELECT id, app_id, connector_type, name, config, enabled, created_at, updated_at
            FROM integrations_connector_configs
            WHERE app_id = $1 AND enabled = TRUE
            ORDER BY connector_type, name
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ConnectorConfig>(
            r#"
            SELECT id, app_id, connector_type, name, config, enabled, created_at, updated_at
            FROM integrations_connector_configs
            WHERE app_id = $1
            ORDER BY connector_type, name
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await
    }
}

/// Fetch the most-recently-created enabled connector config for a given
/// `app_id` + `connector_type` pair.
///
/// Used by the internal carrier-credentials endpoint so that shipping-receiving
/// can look up carrier API credentials without direct DB access.
pub async fn get_config_by_type(
    pool: &PgPool,
    app_id: &str,
    connector_type: &str,
) -> Result<Option<ConnectorConfig>, sqlx::Error> {
    sqlx::query_as::<_, ConnectorConfig>(
        r#"
        SELECT id, app_id, connector_type, name, config, enabled, created_at, updated_at
        FROM integrations_connector_configs
        WHERE app_id = $1 AND connector_type = $2 AND enabled = TRUE
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(connector_type)
    .fetch_optional(pool)
    .await
}

/// Merge `patch` fields into the stored config JSON for a connector config row.
///
/// Used by background pollers (e.g. Amazon SP-API) to persist state (e.g.
/// `last_poll_timestamp`) atomically alongside file_job writes.
pub async fn merge_config_json(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: uuid::Uuid,
    app_id: &str,
    patch: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrations_connector_configs
        SET config = config || $1, updated_at = NOW()
        WHERE id = $2 AND app_id = $3
        "#,
    )
    .bind(patch)
    .bind(id)
    .bind(app_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    connector_type: &str,
    name: &str,
    config: &serde_json::Value,
) -> Result<ConnectorConfig, sqlx::Error> {
    sqlx::query_as(
        r#"
        INSERT INTO integrations_connector_configs
            (app_id, connector_type, name, config, enabled, created_at, updated_at)
        VALUES ($1, $2, $3, $4, TRUE, NOW(), NOW())
        RETURNING id, app_id, connector_type, name, config, enabled, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(connector_type)
    .bind(name)
    .bind(config)
    .fetch_one(&mut **tx)
    .await
}
