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

#[utoipa::path(
    post,
    path = "/api/eco",
    tag = "ECO",
    request_body = CreateEcoRequest,
    responses(
        (status = 201, description = "ECO created (eco_number auto-allocated from Numbering service if omitted)", body = Eco),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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
    let Extension(verified) = claims.as_ref()
        .ok_or_else(|| ApiError::unauthorized("Missing authentication"))?;
    let eco = eco_service::create_eco(
        &state.pool,
        &tenant_id,
        &req,
        Some(&state.numbering),
        auth_header.as_deref(),
        &correlation_id(),
        None,
        verified,
    )
    .await
    .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(eco)))
}

#[utoipa::path(
    get,
    path = "/api/eco/{eco_id}",
    tag = "ECO",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    responses(
        (status = 200, description = "ECO details", body = Eco),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/eco/{eco_id}/submit",
    tag = "ECO Lifecycle",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    request_body = EcoActionRequest,
    responses(
        (status = 200, description = "ECO submitted for review", body = Eco),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid state transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/eco/{eco_id}/approve",
    tag = "ECO Lifecycle",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    request_body = EcoActionRequest,
    responses(
        (status = 200, description = "ECO approved", body = Eco),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid state transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/eco/{eco_id}/reject",
    tag = "ECO Lifecycle",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    request_body = EcoActionRequest,
    responses(
        (status = 200, description = "ECO rejected", body = Eco),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid state transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/eco/{eco_id}/apply",
    tag = "ECO Lifecycle",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    request_body = ApplyEcoRequest,
    responses(
        (status = 200, description = "ECO applied to BOM revisions", body = Eco),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid state transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/eco/{eco_id}/bom-revisions",
    tag = "ECO Links",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    request_body = LinkBomRevisionRequest,
    responses(
        (status = 201, description = "BOM revision linked to ECO", body = EcoBomRevision),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/eco/{eco_id}/doc-revisions",
    tag = "ECO Links",
    params(("eco_id" = Uuid, Path, description = "ECO ID")),
    request_body = LinkDocRevisionRequest,
    responses(
        (status = 201, description = "Doc revision linked to ECO", body = EcoDocRevision),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get,
    path = "/api/eco/{eco_id}/bom-revisions",
    tag = "ECO Links",
    params(("eco_id" = Uuid, Path, description = "ECO ID"), PaginationQuery),
    responses(
        (status = 200, description = "Paginated BOM revision links", body = PaginatedResponse<EcoBomRevision>),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get,
    path = "/api/eco/{eco_id}/doc-revisions",
    tag = "ECO Links",
    params(("eco_id" = Uuid, Path, description = "ECO ID"), PaginationQuery),
    responses(
        (status = 200, description = "Paginated doc revision links", body = PaginatedResponse<EcoDocRevision>),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get,
    path = "/api/eco/history/{part_id}",
    tag = "ECO",
    params(("part_id" = Uuid, Path, description = "Part/item ID"), PaginationQuery),
    responses(
        (status = 200, description = "Paginated ECO history for a part", body = PaginatedResponse<Eco>),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get,
    path = "/api/eco/{eco_id}/audit",
    tag = "ECO",
    params(("eco_id" = Uuid, Path, description = "ECO ID"), PaginationQuery),
    responses(
        (status = 200, description = "Paginated audit trail for the ECO", body = PaginatedResponse<EcoAuditEntry>),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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
