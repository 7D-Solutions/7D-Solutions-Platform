use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;
use axum::Extension;

use crate::domain::bom_service::BomError;
use crate::domain::guards::GuardError;
use crate::domain::models::*;
use crate::domain::mrp_engine;
use crate::AppState;
use platform_sdk::extract_tenant;

fn request_id() -> String {
    Uuid::new_v4().to_string()
}

fn correlation_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn into_api_error(err: BomError) -> ApiError {
    match err {
        BomError::Guard(GuardError::NotFound(msg)) => {
            ApiError::not_found(msg).with_request_id(request_id())
        }
        BomError::Guard(GuardError::Validation(msg)) => {
            ApiError::new(422, "validation_error", msg).with_request_id(request_id())
        }
        BomError::Guard(GuardError::Conflict(msg)) => {
            ApiError::conflict(msg).with_request_id(request_id())
        }
        BomError::Guard(GuardError::CycleDetected) => {
            ApiError::new(422, "cycle_detected", "Cycle detected in BOM structure")
                .with_request_id(request_id())
        }
        BomError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error");
            ApiError::internal("Database error").with_request_id(request_id())
        }
        BomError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error");
            ApiError::internal("Serialization error").with_request_id(request_id())
        }
        BomError::Database(e) => {
            tracing::error!(error = %e, "database error");
            ApiError::internal("Database error").with_request_id(request_id())
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/bom/mrp/explode",
    tag = "BOM MRP",
    request_body = MrpExplodeRequest,
    responses(
        (status = 201, description = "MRP explosion snapshot created", body = MrpSnapshotWithLines),
        (status = 404, description = "BOM not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_mrp_explode(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<MrpExplodeRequest>,
) -> Result<(StatusCode, Json<MrpSnapshotWithLines>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let result = mrp_engine::explode(&state.pool, &tenant_id, &req, &correlation_id(), None)
        .await
        .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(result)))
}

#[utoipa::path(
    get,
    path = "/api/bom/mrp/snapshots/{snapshot_id}",
    tag = "BOM MRP",
    params(("snapshot_id" = Uuid, Path, description = "MRP snapshot ID")),
    responses(
        (status = 200, description = "MRP snapshot with requirement lines", body = MrpSnapshotWithLines),
        (status = 404, description = "Snapshot not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_mrp_snapshot(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(snapshot_id): Path<Uuid>,
) -> Result<Json<MrpSnapshotWithLines>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let result = mrp_engine::get_snapshot(&state.pool, &tenant_id, snapshot_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/api/bom/mrp/snapshots",
    tag = "BOM MRP",
    params(MrpSnapshotListQuery),
    responses(
        (status = 200, description = "Paginated list of MRP snapshots", body = PaginatedResponse<MrpSnapshot>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_mrp_snapshots(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<MrpSnapshotListQuery>,
) -> Result<Json<PaginatedResponse<MrpSnapshot>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = mrp_engine::list_snapshots(&state.pool, &tenant_id, &q)
        .await
        .map_err(into_api_error)?;
    let total = all.len() as i64;
    let start = ((q.page - 1) * q.page_size) as usize;
    let data: Vec<MrpSnapshot> = all.into_iter().skip(start).take(q.page_size as usize).collect();
    Ok(Json(PaginatedResponse::new(data, q.page, q.page_size, total)))
}
