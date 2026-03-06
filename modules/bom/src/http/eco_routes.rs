use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::domain::eco_models::*;
use crate::domain::eco_service;
use crate::http::bom_routes::error_response;
use crate::AppState;

fn correlation_id() -> String {
    Uuid::new_v4().to_string()
}

pub async fn post_eco(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateEcoRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    match eco_service::create_eco(
        &state.pool,
        &tenant_id,
        &req,
        Some(&state.numbering),
        auth_header.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(eco) => (StatusCode::CREATED, Json(json!(eco))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_eco(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::get_eco(&state.pool, &tenant_id, eco_id).await {
        Ok(eco) => Json(json!(eco)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_submit(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<EcoActionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::submit_eco(&state.pool, &tenant_id, eco_id, &req, &correlation_id(), None)
        .await
    {
        Ok(eco) => Json(json!(eco)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_approve(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<EcoActionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::approve_eco(&state.pool, &tenant_id, eco_id, &req, &correlation_id(), None)
        .await
    {
        Ok(eco) => Json(json!(eco)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_reject(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<EcoActionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::reject_eco(&state.pool, &tenant_id, eco_id, &req, &correlation_id(), None)
        .await
    {
        Ok(eco) => Json(json!(eco)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_apply(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<ApplyEcoRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::apply_eco(&state.pool, &tenant_id, eco_id, &req, &correlation_id(), None)
        .await
    {
        Ok(eco) => Json(json!(eco)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_link_bom_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<LinkBomRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::link_bom_revision(&state.pool, &tenant_id, eco_id, &req).await {
        Ok(link) => (StatusCode::CREATED, Json(json!(link))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_link_doc_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<LinkDocRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::link_doc_revision(&state.pool, &tenant_id, eco_id, &req).await {
        Ok(link) => (StatusCode::CREATED, Json(json!(link))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_bom_revision_links(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::list_bom_revision_links(&state.pool, &tenant_id, eco_id).await {
        Ok(links) => Json(json!(links)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_doc_revision_links(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::list_doc_revision_links(&state.pool, &tenant_id, eco_id).await {
        Ok(links) => Json(json!(links)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_eco_history_for_part(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(part_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::eco_history_for_part(&state.pool, &tenant_id, part_id).await {
        Ok(ecos) => Json(json!(ecos)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_eco_audit(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match eco_service::get_audit_trail(&state.pool, &tenant_id, eco_id).await {
        Ok(entries) => Json(json!(entries)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
