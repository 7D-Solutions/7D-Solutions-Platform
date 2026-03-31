//! HTTP handlers for asset disposals and impairments.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::disposals::{DisposalService, DisposeAssetRequest};
use crate::AppState;

use super::helpers::tenant::{extract_tenant, with_request_id};

/// POST /api/fixed-assets/disposals — Dispose or impair an asset. Idempotent.
pub async fn dispose_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<DisposeAssetRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match DisposalService::dispose(&state.pool, &req).await {
        Ok(disposal) => (StatusCode::CREATED, Json(disposal)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/fixed-assets/disposals — List all disposals.
pub async fn list_disposals(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match DisposalService::list(&state.pool, &tenant_id).await {
        Ok(disposals) => {
            let total = disposals.len() as i64;
            let resp = PaginatedResponse::new(disposals, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/fixed-assets/disposals/:id — Fetch a single disposal.
pub async fn get_disposal(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match DisposalService::get(&state.pool, id, &tenant_id).await {
        Ok(Some(disposal)) => Json(disposal).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Disposal {} not found", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
