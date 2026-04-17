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

use crate::domain::signoffs::{service, ListSignoffsQuery, RecordSignoffRequest};
use crate::AppState;

pub async fn record_signoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<RecordSignoffRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let user_id = claims.as_ref().map(|c| c.user_id).unwrap_or(Uuid::nil());

    match service::record_signoff(&state.pool, &tenant_id, user_id, req).await {
        Ok(s) => {
            state.metrics.signoffs_recorded.inc();
            (StatusCode::CREATED, Json(s)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn list_signoffs(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListSignoffsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_signoffs(&state.pool, &tenant_id, query).await {
        Ok(ss) => Json(ss).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn get_signoff(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(signoff_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_signoff(&state.pool, signoff_id, &tenant_id).await {
        Ok(s) => Json(s).into_response(),
        Err(e) => e.into_response(),
    }
}
