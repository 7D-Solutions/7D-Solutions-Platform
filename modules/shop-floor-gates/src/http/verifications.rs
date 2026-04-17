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

use crate::domain::verifications::{
    service, CreateVerificationRequest, ListVerificationsQuery, OperatorConfirmRequest,
    SkipVerificationRequest, VerifyRequest,
};
use crate::AppState;

pub async fn create_verification(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateVerificationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::create_verification(&state.pool, &tenant_id, user_id, req).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn list_verifications(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListVerificationsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_verifications(&state.pool, &tenant_id, query).await {
        Ok(vs) => Json(vs).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn get_verification(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(verification_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_verification(&state.pool, verification_id, &tenant_id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn operator_confirm(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(verification_id): Path<Uuid>,
    Json(req): Json<OperatorConfirmRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::operator_confirm(&state.pool, &tenant_id, verification_id, user_id, req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn verify(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(verification_id): Path<Uuid>,
    Json(req): Json<VerifyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::verify(&state.pool, &tenant_id, verification_id, user_id, req).await {
        Ok(v) => {
            state.metrics.verifications_completed.inc();
            Json(v).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn skip_verification(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(verification_id): Path<Uuid>,
    Json(req): Json<SkipVerificationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::skip_verification(&state.pool, &tenant_id, verification_id, user_id, req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}
