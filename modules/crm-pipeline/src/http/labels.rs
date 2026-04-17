//! HTTP handlers for label tables — 4 list endpoints per spec §4.6.

use axum::{
    extract::State,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;

use crate::domain::labels;
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    get, path = "/api/crm-pipeline/status-labels", tag = "Labels",
    responses((status = 200, body = Vec<crate::domain::labels::Label>)),
    security(("bearer" = [])),
)]
pub async fn list_status_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match labels::list_status_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/source-labels", tag = "Labels",
    responses((status = 200, body = Vec<crate::domain::labels::Label>)),
    security(("bearer" = [])),
)]
pub async fn list_source_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match labels::list_source_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/type-labels", tag = "Labels",
    responses((status = 200, body = Vec<crate::domain::labels::Label>)),
    security(("bearer" = [])),
)]
pub async fn list_type_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match labels::list_type_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/priority-labels", tag = "Labels",
    responses((status = 200, body = Vec<crate::domain::labels::Label>)),
    security(("bearer" = [])),
)]
pub async fn list_priority_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match labels::list_priority_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
