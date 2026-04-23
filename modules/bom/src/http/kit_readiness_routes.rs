use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::mrp_routes::into_api_error;
use crate::domain::kit_readiness_engine;
use crate::domain::models::*;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    post,
    path = "/api/bom/kit-readiness/check",
    tag = "BOM Kit Readiness",
    request_body = KitReadinessCheckRequest,
    responses(
        (status = 201, description = "Kit readiness snapshot created", body = KitReadinessResult),
        (status = 404, description = "BOM not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 503, description = "Inventory service unavailable", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_kit_readiness_check(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<KitReadinessCheckRequest>,
) -> Result<(StatusCode, Json<KitReadinessResult>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let claims_ref = claims.as_deref();
    let result = kit_readiness_engine::check(
        &state.pool,
        &tenant_id,
        &req,
        &state.inventory,
        claims_ref,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(result)))
}

#[utoipa::path(
    get,
    path = "/api/bom/kit-readiness/snapshots/{snapshot_id}",
    tag = "BOM Kit Readiness",
    params(("snapshot_id" = Uuid, Path, description = "Kit readiness snapshot ID")),
    responses(
        (status = 200, description = "Kit readiness snapshot with per-component lines", body = KitReadinessResult),
        (status = 404, description = "Snapshot not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_kit_readiness_snapshot(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(snapshot_id): Path<Uuid>,
) -> Result<Json<KitReadinessResult>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let result = kit_readiness_engine::get_snapshot(&state.pool, &tenant_id, snapshot_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(result))
}
