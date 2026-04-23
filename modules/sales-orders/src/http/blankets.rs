//! HTTP handlers for blanket orders — CRUD + activate.

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
use uuid::Uuid;

use crate::domain::blankets::{
    service, ActivateBlanketRequest, CreateBlanketLineRequest, CreateBlanketRequest,
    ListBlanketsQuery, UpdateBlanketRequest,
};
use crate::AppState;

#[utoipa::path(
    post, path = "/api/so/blankets", tag = "BlanketOrders",
    request_body = CreateBlanketRequest,
    responses(
        (status = 201, description = "Blanket order created", body = crate::domain::blankets::BlanketOrder),
        (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_blanket(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateBlanketRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };
    let created_by = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match service::create_blanket(&state.pool, &tenant_id, &created_by, req).await {
        Ok(b) => (StatusCode::CREATED, Json(b)).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(ApiError::from(e))).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/so/blankets", tag = "BlanketOrders",
    responses((status = 200, description = "Blanket list", body = Vec<crate::domain::blankets::BlanketOrder>)),
    security(("bearer" = [])),
)]
pub async fn list_blankets(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListBlanketsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::list_blankets(&state.pool, &tenant_id, &query).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError::from(e))).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/so/blankets/{blanket_id}", tag = "BlanketOrders",
    responses(
        (status = 200, description = "Blanket with lines", body = crate::domain::blankets::BlanketOrderWithLines),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_blanket(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(blanket_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::get_blanket_with_lines(&state.pool, &tenant_id, blanket_id).await {
        Ok(b) => Json(b).into_response(),
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

#[utoipa::path(
    put, path = "/api/so/blankets/{blanket_id}", tag = "BlanketOrders",
    request_body = UpdateBlanketRequest,
    responses(
        (status = 200, description = "Blanket updated", body = crate::domain::blankets::BlanketOrder),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_blanket(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(blanket_id): Path<Uuid>,
    Json(req): Json<UpdateBlanketRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::update_blanket(&state.pool, &tenant_id, blanket_id, req).await {
        Ok(b) => Json(b).into_response(),
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
    post, path = "/api/so/blankets/{blanket_id}/activate", tag = "BlanketOrders",
    request_body = ActivateBlanketRequest,
    responses(
        (status = 200, description = "Blanket activated", body = crate::domain::blankets::BlanketOrder),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn activate_blanket(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(blanket_id): Path<Uuid>,
    Json(req): Json<ActivateBlanketRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::activate_blanket(&state.pool, &tenant_id, blanket_id, req).await {
        Ok(b) => Json(b).into_response(),
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
    post, path = "/api/so/blankets/{blanket_id}/lines", tag = "BlanketOrders",
    request_body = CreateBlanketLineRequest,
    responses(
        (status = 201, description = "Blanket line added", body = crate::domain::blankets::BlanketOrderLine),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_blanket_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(blanket_id): Path<Uuid>,
    Json(req): Json<CreateBlanketLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::add_blanket_line(&state.pool, &tenant_id, blanket_id, req).await {
        Ok(line) => (StatusCode::CREATED, Json(line)).into_response(),
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
