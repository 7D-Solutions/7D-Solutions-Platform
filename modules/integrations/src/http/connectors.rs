//! HTTP handlers for connector registration and test-action dispatch.
//!
//! Routes:
//!   GET  /api/integrations/connectors/types        — list registered connector types
//!   POST /api/integrations/connectors              — register a connector config
//!   GET  /api/integrations/connectors              — list tenant's connector configs
//!   GET  /api/integrations/connectors/:id          — get one config
//!   POST /api/integrations/connectors/:id/test     — run test action

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::connectors::{
    all_connectors, service, ConnectorCapabilities, ConnectorConfig, ConnectorError,
    RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
};
use crate::AppState;

// ============================================================================
// Error helpers
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    fn new(error: &str, message: &str) -> Self {
        Self { error: error.to_string(), message: message.to_string() }
    }
}

fn connector_error_response(e: ConnectorError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        ConnectorError::UnknownType(t) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("unknown_connector_type", &format!("Unknown connector type: {}", t))),
        ),
        ConnectorError::InvalidConfig(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("invalid_config", &msg)),
        ),
        ConnectorError::ActionFailed(msg) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody::new("action_failed", &msg)),
        ),
        ConnectorError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", &format!("Connector config {} not found", id))),
        ),
        ConnectorError::Database(e) => {
            tracing::error!("Connector DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

fn app_id_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_app_id", "X-App-Id header is required")),
            )
        })
}

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub enabled_only: bool,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /api/integrations/connectors/types — list all registered connector types and their capabilities
pub async fn list_connector_types() -> Json<Vec<ConnectorCapabilities>> {
    Json(all_connectors())
}

/// POST /api/integrations/connectors — register a connector config for this tenant
pub async fn register_connector(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RegisterConnectorRequest>,
) -> Result<(StatusCode, Json<ConnectorConfig>), (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let created = service::register_connector(&state.pool, &app_id, &req, correlation_id)
        .await
        .map_err(connector_error_response)?;

    Ok((StatusCode::CREATED, Json(created)))
}

/// GET /api/integrations/connectors — list this tenant's connector configs
pub async fn list_connectors(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ConnectorConfig>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let configs =
        service::list_connector_configs(&state.pool, &app_id, q.enabled_only)
            .await
            .map_err(connector_error_response)?;

    Ok(Json(configs))
}

/// GET /api/integrations/connectors/:id — fetch a single connector config
pub async fn get_connector(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<ConnectorConfig>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let config = service::get_connector_config(&state.pool, &app_id, id)
        .await
        .map_err(connector_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new("not_found", &format!("Connector config {} not found", id))),
            )
        })?;

    Ok(Json(config))
}

/// POST /api/integrations/connectors/:id/test — run the connector's test action
pub async fn run_connector_test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<RunTestActionRequest>,
) -> Result<Json<TestActionResult>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let result = service::run_test_action(&state.pool, &app_id, id, &req)
        .await
        .map_err(connector_error_response)?;

    Ok(Json(result))
}
