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
