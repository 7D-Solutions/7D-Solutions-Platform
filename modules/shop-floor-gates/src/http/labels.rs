use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::labels::{service, UpsertLabelRequest};
use crate::AppState;

pub async fn upsert_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(table): Path<String>,
    Json(req): Json<UpsertLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::upsert_label(&state.pool, &table, &tenant_id, req).await {
        Ok(label) => (StatusCode::OK, Json(label)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn list_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(table): Path<String>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_labels(&state.pool, &table, &tenant_id).await {
        Ok(labels) => Json(labels).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn delete_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((table, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::delete_label(&state.pool, &table, id, &tenant_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => e.into_response(),
    }
}
