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
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
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

fn connector_error(e: ConnectorError) -> ApiError {
    match e {
        ConnectorError::UnknownType(t) => {
            ApiError::new(422, "unknown_connector_type", format!("Unknown connector type: {}", t))
        }
        ConnectorError::InvalidConfig(msg) => {
            ApiError::new(422, "invalid_config", msg)
        }
        ConnectorError::ActionFailed(msg) => {
            ApiError::new(502, "action_failed", msg)
        }
        ConnectorError::NotFound(id) => {
            ApiError::not_found(format!("Connector config {} not found", id))
        }
        ConnectorError::Database(e) => {
            tracing::error!("Connector DB error: {}", e);
            ApiError::internal("Internal database error")
        }
    }
}

fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized("Missing or invalid authentication")),
    }
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

#[utoipa::path(
    get,
    path = "/api/integrations/connectors/types",
    responses(
        (status = 200, description = "List of connector types", body = PaginatedResponse<ConnectorCapabilities>),
    ),
    security(("bearer" = [])),
    tag = "Connectors"
)]
/// GET /api/integrations/connectors/types — list all registered connector types and their capabilities
pub async fn list_connector_types() -> impl IntoResponse {
    let types = all_connectors();
    let total = types.len() as i64;
    let resp = PaginatedResponse::new(types, 1, total.max(1), total);
    Json(resp).into_response()
}

#[utoipa::path(
    post,
    path = "/api/integrations/connectors",
    request_body = RegisterConnectorRequest,
    responses(
        (status = 201, description = "Connector registered", body = ConnectorConfig),
        (status = 422, description = "Invalid config or unknown type"),
    ),
    security(("bearer" = [])),
    tag = "Connectors"
)]
/// POST /api/integrations/connectors — register a connector config for this tenant
pub async fn register_connector(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<RegisterConnectorRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::register_connector(&state.pool, &app_id, &req, correlation_id).await {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => connector_error(e).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/integrations/connectors",
    params(("enabled_only" = bool, Query, description = "Filter to enabled connectors only")),
    responses(
        (status = 200, description = "List of connector configs", body = PaginatedResponse<ConnectorConfig>),
    ),
    security(("bearer" = [])),
    tag = "Connectors"
)]
/// GET /api/integrations/connectors — list this tenant's connector configs
pub async fn list_connectors(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_connector_configs(&state.pool, &app_id, q.enabled_only).await {
        Ok(configs) => {
            let total = configs.len() as i64;
            let resp = PaginatedResponse::new(configs, 1, total.max(1), total);
            Json(resp).into_response()
        }
        Err(e) => connector_error(e).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/integrations/connectors/{id}",
    params(("id" = Uuid, Path, description = "Connector config ID")),
    responses(
        (status = 200, description = "Connector config", body = ConnectorConfig),
        (status = 404, description = "Not found"),
    ),
    security(("bearer" = [])),
    tag = "Connectors"
)]
/// GET /api/integrations/connectors/:id — fetch a single connector config
pub async fn get_connector(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_connector_config(&state.pool, &app_id, id).await {
        Ok(Some(config)) => Json(config).into_response(),
        Ok(None) => {
            ApiError::not_found(format!("Connector config {} not found", id)).into_response()
        }
        Err(e) => connector_error(e).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/integrations/connectors/{id}/test",
    params(("id" = Uuid, Path, description = "Connector config ID")),
    request_body = RunTestActionRequest,
    responses(
        (status = 200, description = "Test result", body = TestActionResult),
        (status = 404, description = "Not found"),
        (status = 502, description = "Action failed"),
    ),
    security(("bearer" = [])),
    tag = "Connectors"
)]
/// POST /api/integrations/connectors/:id/test — run the connector's test action
pub async fn run_connector_test(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RunTestActionRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::run_test_action(&state.pool, &app_id, id, &req).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => connector_error(e).into_response(),
    }
}
