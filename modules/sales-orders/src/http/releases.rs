//! HTTP handlers for blanket order releases.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::blankets::{service, CreateReleaseRequest};
use crate::AppState;

#[utoipa::path(
    post, path = "/api/so/blankets/{blanket_id}/releases", tag = "BlanketReleases",
    request_body = CreateReleaseRequest,
    responses(
        (status = 201, description = "Release created", body = crate::domain::blankets::BlanketOrderRelease),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_release(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(blanket_id): Path<Uuid>,
    Json(req): Json<CreateReleaseRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::create_release(&state.pool, &tenant_id, blanket_id, req).await {
        Ok(r) => (StatusCode::CREATED, Json(r)).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    get, path = "/api/so/blankets/{blanket_id}/lines/{line_id}/releases", tag = "BlanketReleases",
    responses(
        (status = 200, description = "Releases for line", body = Vec<crate::domain::blankets::BlanketOrderRelease>),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_releases(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((blanket_id, line_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::get_releases_for_blanket(&state.pool, &tenant_id, blanket_id, line_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(api_err),
            )
                .into_response()
        }
    }
}
