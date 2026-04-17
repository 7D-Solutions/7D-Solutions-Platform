//! HTTP handlers for status label management.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use std::sync::Arc;

use crate::domain::labels::{service, ListLabelsQuery, UpsertLabelRequest};
use crate::AppState;

#[utoipa::path(
    get, path = "/api/so/labels", tag = "SalesOrderLabels",
    responses((status = 200, description = "Label list", body = Vec<crate::domain::labels::StatusLabel>)),
    security(("bearer" = [])),
)]
pub async fn list_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListLabelsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::list_labels(&state.pool, &tenant_id, &query).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError::from(e))).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/so/labels/{label_type}/{status_key}", tag = "SalesOrderLabels",
    request_body = UpsertLabelRequest,
    responses(
        (status = 200, description = "Label upserted", body = crate::domain::labels::StatusLabel),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn upsert_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((label_type, status_key)): Path<(String, String)>,
    Json(req): Json<UpsertLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::upsert_label(&state.pool, &tenant_id, &label_type, &status_key, req).await {
        Ok(label) => Json(label).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError::from(e))).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/so/labels/{label_type}/{status_key}", tag = "SalesOrderLabels",
    responses(
        (status = 204, description = "Label deleted"),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn delete_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((label_type, status_key)): Path<(String, String)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::delete_label(&state.pool, &tenant_id, &label_type, &status_key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code()).unwrap_or(StatusCode::NOT_FOUND),
                Json(api_err),
            )
                .into_response()
        }
    }
}
