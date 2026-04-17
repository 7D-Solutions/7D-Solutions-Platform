//! HTTP handlers for status labels and service-type labels.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;

use crate::domain::{models::*, repo};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

pub async fn list_status_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_status_labels(&state.pool, &tenant_id).await {
        Ok(labels) => Json(labels).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

pub async fn upsert_status_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(canonical_status): Path<String>,
    Json(req): Json<UpsertStatusLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::upsert_status_label(&state.pool, &tenant_id, &canonical_status, &req).await {
        Ok(label) => Json(label).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

pub async fn list_service_type_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_service_type_labels(&state.pool, &tenant_id).await {
        Ok(labels) => Json(labels).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

pub async fn upsert_service_type_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(service_type): Path<String>,
    Json(req): Json<UpsertServiceTypeLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::upsert_service_type_label(&state.pool, &tenant_id, &service_type, &req).await {
        Ok(label) => Json(label).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
