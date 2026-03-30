use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use platform_http_contracts::PaginatedResponse;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::domain::eco_models::*;
use crate::domain::models::PaginationQuery;
use crate::domain::eco_service;
use crate::http::bom_routes::{into_api_error, paginate};
use crate::AppState;
use platform_http_contracts::ApiError;

fn correlation_id() -> String {
    Uuid::new_v4().to_string()
}

pub async fn post_eco(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateEcoRequest>,
) -> Result<(StatusCode, Json<Eco>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let eco = eco_service::create_eco(
        &state.pool,
        &tenant_id,
        &req,
        Some(&state.numbering),
        auth_header.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(eco)))
}

pub async fn get_eco(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
) -> Result<Json<Eco>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let eco = eco_service::get_eco(&state.pool, &tenant_id, eco_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(eco))
}

pub async fn post_submit(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<EcoActionRequest>,
) -> Result<Json<Eco>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let eco = eco_service::submit_eco(
        &state.pool,
        &tenant_id,
        eco_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok(Json(eco))
}

pub async fn post_approve(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<EcoActionRequest>,
) -> Result<Json<Eco>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let eco = eco_service::approve_eco(
        &state.pool,
        &tenant_id,
        eco_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok(Json(eco))
}

pub async fn post_reject(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<EcoActionRequest>,
) -> Result<Json<Eco>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let eco = eco_service::reject_eco(
        &state.pool,
        &tenant_id,
        eco_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok(Json(eco))
}

pub async fn post_apply(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<ApplyEcoRequest>,
) -> Result<Json<Eco>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let eco = eco_service::apply_eco(
        &state.pool,
        &tenant_id,
        eco_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok(Json(eco))
}

pub async fn post_link_bom_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<LinkBomRevisionRequest>,
) -> Result<(StatusCode, Json<EcoBomRevision>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let link = eco_service::link_bom_revision(&state.pool, &tenant_id, eco_id, &req)
        .await
        .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(link)))
}

pub async fn post_link_doc_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Json(req): Json<LinkDocRevisionRequest>,
) -> Result<(StatusCode, Json<EcoDocRevision>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let link = eco_service::link_doc_revision(&state.pool, &tenant_id, eco_id, &req)
        .await
        .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(link)))
}

pub async fn get_bom_revision_links(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<EcoBomRevision>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = eco_service::list_bom_revision_links(&state.pool, &tenant_id, eco_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}

pub async fn get_doc_revision_links(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<EcoDocRevision>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = eco_service::list_doc_revision_links(&state.pool, &tenant_id, eco_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}

pub async fn get_eco_history_for_part(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(part_id): Path<Uuid>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<Eco>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = eco_service::eco_history_for_part(&state.pool, &tenant_id, part_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}

pub async fn get_eco_audit(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(eco_id): Path<Uuid>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<EcoAuditEntry>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = eco_service::get_audit_trail(&state.pool, &tenant_id, eco_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}
