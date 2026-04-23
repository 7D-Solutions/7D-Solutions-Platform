use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::handoffs::{
    service, AcceptHandoffRequest, CancelHandoffRequest, InitiateHandoffRequest, ListHandoffsQuery,
    RejectHandoffRequest,
};
use crate::AppState;

pub async fn initiate_handoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<InitiateHandoffRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::initiate_handoff(&state.pool, &tenant_id, user_id, req).await {
        Ok(h) => {
            state.metrics.handoffs_initiated.inc();
            (StatusCode::CREATED, Json(h)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn list_handoffs(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListHandoffsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_handoffs(&state.pool, &tenant_id, query).await {
        Ok(hs) => Json(hs).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn get_handoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(handoff_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_handoff(&state.pool, handoff_id, &tenant_id).await {
        Ok(h) => Json(h).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn accept_handoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(handoff_id): Path<Uuid>,
    Json(req): Json<AcceptHandoffRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::accept_handoff(&state.pool, &tenant_id, handoff_id, user_id, req).await {
        Ok(h) => {
            state.metrics.handoffs_accepted.inc();
            Json(h).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn reject_handoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(handoff_id): Path<Uuid>,
    Json(req): Json<RejectHandoffRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::reject_handoff(&state.pool, &tenant_id, handoff_id, user_id, req).await {
        Ok(h) => Json(h).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn cancel_handoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(handoff_id): Path<Uuid>,
    Json(req): Json<CancelHandoffRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::cancel_handoff(&state.pool, &tenant_id, handoff_id, user_id, req).await {
        Ok(h) => Json(h).into_response(),
        Err(e) => e.into_response(),
    }
}
