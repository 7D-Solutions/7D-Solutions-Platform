//! Lot genealogy HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/lots/split                   — split a lot
//!   POST /api/inventory/lots/merge                   — merge lots
//!   GET  /api/inventory/lots/{lot_id}/children       — forward trace
//!   GET  /api/inventory/lots/{lot_id}/parents        — reverse trace

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::genealogy::{
        children_of, parents_of, process_merge, process_split, GenealogyResult, LotMergeRequest,
        LotSplitRequest,
    },
    AppState,
};

#[utoipa::path(
    post,
    path = "/api/inventory/lots/split",
    tag = "Lot Genealogy",
    request_body = LotSplitRequest,
    responses(
        (status = 201, description = "Lot split completed", body = GenealogyResult),
        (status = 200, description = "Idempotency replay", body = GenealogyResult),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_lot_split(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<LotSplitRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match process_split(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/lots/merge",
    tag = "Lot Genealogy",
    request_body = LotMergeRequest,
    responses(
        (status = 201, description = "Lot merge completed", body = GenealogyResult),
        (status = 200, description = "Idempotency replay", body = GenealogyResult),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_lot_merge(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<LotMergeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match process_merge(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/lots/{lot_id}/children",
    tag = "Lot Genealogy",
    params(("lot_id" = Uuid, Path, description = "Lot ID")),
    responses(
        (status = 200, description = "Child genealogy edges", body = serde_json::Value),
    ),
    security(("bearer" = [])),
)]
pub async fn get_lot_children(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(lot_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match children_of(&state.pool, &tenant_id, lot_id).await {
        Ok(edges) => (StatusCode::OK, Json(json!({ "edges": edges }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, lot_id = %lot_id, "database error querying children");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/lots/{lot_id}/parents",
    tag = "Lot Genealogy",
    params(("lot_id" = Uuid, Path, description = "Lot ID")),
    responses(
        (status = 200, description = "Parent genealogy edges", body = serde_json::Value),
    ),
    security(("bearer" = [])),
)]
pub async fn get_lot_parents(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(lot_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match parents_of(&state.pool, &tenant_id, lot_id).await {
        Ok(edges) => (StatusCode::OK, Json(json!({ "edges": edges }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, lot_id = %lot_id, "database error querying parents");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}
