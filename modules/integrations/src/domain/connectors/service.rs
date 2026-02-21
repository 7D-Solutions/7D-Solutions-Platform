//! Connector config service — CRUD for `integrations_connector_configs`.
//!
//! Follows Guard→Mutation→Outbox atomicity.  The outbox event for connector
//! registration is intentionally lightweight (`connector.registered`) so
//! downstream systems can react without polling.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::outbox::enqueue_event_tx;

use super::{
    get_connector, ConnectorConfig, ConnectorError, RegisterConnectorRequest, TestActionResult,
    RunTestActionRequest,
};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single connector config by id, scoped to app_id.
pub async fn get_connector_config(
    pool: &PgPool,
    app_id: &str,
    id: Uuid,
) -> Result<Option<ConnectorConfig>, ConnectorError> {
    let row = sqlx::query_as::<_, ConnectorConfig>(
        r#"
        SELECT id, app_id, connector_type, name, config, enabled, created_at, updated_at
        FROM integrations_connector_configs
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// List all connector configs for a tenant, optionally filtered to enabled only.
pub async fn list_connector_configs(
    pool: &PgPool,
    app_id: &str,
    enabled_only: bool,
) -> Result<Vec<ConnectorConfig>, ConnectorError> {
    let rows = if enabled_only {
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
        .await?
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
        .await?
    };
    Ok(rows)
}

// ============================================================================
// Writes
// ============================================================================

/// Register a new connector config.
///
/// Guard: connector_type must be known; config is validated by the connector's
/// own schema before persisting.
///
/// Emits `connector.registered` via the transactional outbox.
pub async fn register_connector(
    pool: &PgPool,
    app_id: &str,
    req: &RegisterConnectorRequest,
    _correlation_id: String,
) -> Result<ConnectorConfig, ConnectorError> {
    // Guard: connector type must be registered
    let connector = get_connector(&req.connector_type)
        .ok_or_else(|| ConnectorError::UnknownType(req.connector_type.clone()))?;

    if req.name.trim().is_empty() {
        return Err(ConnectorError::InvalidConfig("name cannot be empty".to_string()));
    }

    let config = req.config.clone().unwrap_or(serde_json::Value::Object(Default::default()));

    // Guard: validate config against connector schema
    connector.validate_config(&config)?;

    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let row: ConnectorConfig = sqlx::query_as(
        r#"
        INSERT INTO integrations_connector_configs
            (app_id, connector_type, name, config, enabled, created_at, updated_at)
        VALUES ($1, $2, $3, $4, TRUE, NOW(), NOW())
        RETURNING id, app_id, connector_type, name, config, enabled, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.connector_type.trim())
    .bind(req.name.trim())
    .bind(&config)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
            ConnectorError::InvalidConfig(format!(
                "A '{}' connector named '{}' already exists for this tenant",
                req.connector_type, req.name
            ))
        } else {
            ConnectorError::Database(e)
        }
    })?;

    // Outbox: connector.registered
    let payload = serde_json::json!({
        "connector_id": row.id,
        "app_id": app_id,
        "connector_type": row.connector_type,
        "name": row.name,
        "registered_at": Utc::now(),
    });
    enqueue_event_tx(
        &mut tx,
        event_id,
        "connector.registered",
        "connector",
        &row.id.to_string(),
        app_id,
        &payload,
    )
    .await?;

    tx.commit().await?;
    Ok(row)
}

// ============================================================================
// Test action dispatch
// ============================================================================

/// Run the test action for an existing connector config.
///
/// Guard: the config must exist and belong to `app_id`.
/// Dispatches to the connector implementation — result is deterministic.
pub async fn run_test_action(
    pool: &PgPool,
    app_id: &str,
    connector_id: Uuid,
    req: &RunTestActionRequest,
) -> Result<TestActionResult, ConnectorError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(ConnectorError::InvalidConfig(
            "idempotency_key cannot be empty".to_string(),
        ));
    }

    // Guard: fetch config
    let row = get_connector_config(pool, app_id, connector_id)
        .await?
        .ok_or_else(|| ConnectorError::NotFound(connector_id.to_string()))?;

    if !row.enabled {
        return Err(ConnectorError::InvalidConfig(
            "connector is disabled".to_string(),
        ));
    }

    // Dispatch to implementation
    let connector = get_connector(&row.connector_type)
        .ok_or_else(|| ConnectorError::UnknownType(row.connector_type.clone()))?;

    connector.run_test_action(&row.config, &req.idempotency_key)
}
