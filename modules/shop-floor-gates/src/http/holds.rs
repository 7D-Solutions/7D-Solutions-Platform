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

use crate::domain::holds::{
    service, CancelHoldRequest, ListHoldsQuery, PlaceHoldRequest, ReleaseHoldRequest,
};
use crate::AppState;

pub async fn place_hold(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<PlaceHoldRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::place_hold(&state.pool, &tenant_id, user_id, req).await {
        Ok(hold) => {
            state.metrics.holds_placed.inc();
            (StatusCode::CREATED, Json(hold)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn list_holds(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListHoldsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_holds(&state.pool, &tenant_id, query).await {
        Ok(holds) => Json(holds).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn get_hold(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(hold_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_hold(&state.pool, hold_id, &tenant_id).await {
        Ok(hold) => Json(hold).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn release_hold(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(hold_id): Path<Uuid>,
    Json(req): Json<ReleaseHoldRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());
    let roles = claims.as_ref().map(|c| c.roles.clone()).unwrap_or_default();
    let role = roles.first().map(|r| r.as_str()).unwrap_or("operator");

    match service::release_hold(&state.pool, &tenant_id, hold_id, user_id, req, role, false).await {
        Ok(hold) => {
            state.metrics.holds_released.inc();
            Json(hold).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn cancel_hold(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(hold_id): Path<Uuid>,
    Json(req): Json<CancelHoldRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::cancel_hold(&state.pool, &tenant_id, hold_id, user_id, req).await {
        Ok(hold) => Json(hold).into_response(),
        Err(e) => e.into_response(),
    }
}

// Convenience: count active holds for a work order
pub async fn active_hold_count(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(work_order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match crate::domain::holds::repo::count_active_holds_for_work_order(
        &state.pool,
        work_order_id,
        &tenant_id,
    )
    .await
    {
        Ok(count) => Json(serde_json::json!({ "active_holds": count })).into_response(),
        Err(e) => ApiError::internal(e.to_string()).into_response(),
    }
}
