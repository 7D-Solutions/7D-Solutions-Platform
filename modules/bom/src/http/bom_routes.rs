use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::domain::bom_service::{self, BomError};
use crate::domain::guards::GuardError;
use crate::domain::models::*;
use crate::AppState;

// ============================================================================
// Error mapping → ApiError
// ============================================================================

pub fn into_api_error(err: BomError) -> ApiError {
    match err {
        BomError::Guard(GuardError::NotFound(msg)) => {
            ApiError::not_found(msg).with_request_id(request_id())
        }
        BomError::Guard(GuardError::Validation(msg)) => {
            ApiError::new(422, "validation_error", msg).with_request_id(request_id())
        }
        BomError::Guard(GuardError::Conflict(msg)) => {
            ApiError::conflict(msg).with_request_id(request_id())
        }
        BomError::Guard(GuardError::CycleDetected) => {
            ApiError::new(422, "cycle_detected", "Cycle detected in BOM structure")
                .with_request_id(request_id())
        }
        BomError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error");
            ApiError::internal("Database error").with_request_id(request_id())
        }
        BomError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error");
            ApiError::internal("Serialization error").with_request_id(request_id())
        }
        BomError::Database(ref e) => {
            if let sqlx::Error::Database(dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return ApiError::conflict(
                        "A record with this identifier already exists",
                    )
                    .with_request_id(request_id());
                }
                if dbe.code().as_deref() == Some("23P01") {
                    return ApiError::conflict(
                        "Effective date range overlaps with an existing revision",
                    )
                    .with_request_id(request_id());
                }
            }
            tracing::error!(error = %e, "database error");
            ApiError::internal("Database error").with_request_id(request_id())
        }
    }
}

fn request_id() -> String {
    Uuid::new_v4().to_string()
}

fn correlation_id() -> String {
    Uuid::new_v4().to_string()
}

/// Paginate a pre-fetched collection.
pub fn paginate<T: utoipa::ToSchema>(items: Vec<T>, pq: &PaginationQuery) -> PaginatedResponse<T> {
    let total = items.len() as i64;
    let start = ((pq.page - 1) * pq.page_size) as usize;
    let data: Vec<T> = items.into_iter().skip(start).take(pq.page_size as usize).collect();
    PaginatedResponse::new(data, pq.page, pq.page_size, total)
}

// ============================================================================
// BOM Header
// ============================================================================

pub async fn list_boms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<BomHeader>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = bom_service::list_boms(&state.pool, &tenant_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}

pub async fn post_bom(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateBomRequest>,
) -> Result<(StatusCode, Json<BomHeader>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let header =
        bom_service::create_bom(&state.pool, &tenant_id, &req, &correlation_id(), None)
            .await
            .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(header)))
}

pub async fn get_bom(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
) -> Result<Json<BomHeader>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let header = bom_service::get_bom(&state.pool, &tenant_id, bom_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(header))
}

pub async fn get_bom_by_part_id(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(part_id): Path<Uuid>,
) -> Result<Json<BomHeader>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let header = bom_service::get_bom_by_part_id(&state.pool, &tenant_id, part_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(header))
}

// ============================================================================
// Revisions
// ============================================================================

pub async fn post_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
    Json(req): Json<CreateRevisionRequest>,
) -> Result<(StatusCode, Json<BomRevision>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let rev = bom_service::create_revision(
        &state.pool,
        &tenant_id,
        bom_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(rev)))
}

pub async fn list_revisions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<BomRevision>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = bom_service::list_revisions(&state.pool, &tenant_id, bom_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}

pub async fn post_effectivity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(revision_id): Path<Uuid>,
    Json(req): Json<SetEffectivityRequest>,
) -> Result<Json<BomRevision>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let rev = bom_service::set_effectivity(
        &state.pool,
        &tenant_id,
        revision_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok(Json(rev))
}

// ============================================================================
// Lines
// ============================================================================

pub async fn post_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(revision_id): Path<Uuid>,
    Json(req): Json<AddLineRequest>,
) -> Result<(StatusCode, Json<BomLine>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let line = bom_service::add_line(
        &state.pool,
        &tenant_id,
        revision_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok((StatusCode::CREATED, Json(line)))
}

pub async fn put_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(line_id): Path<Uuid>,
    Json(req): Json<UpdateLineRequest>,
) -> Result<Json<BomLine>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let line = bom_service::update_line(
        &state.pool,
        &tenant_id,
        line_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    .map_err(into_api_error)?;
    Ok(Json(line))
}

pub async fn delete_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(line_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    bom_service::remove_line(&state.pool, &tenant_id, line_id, &correlation_id(), None)
        .await
        .map_err(into_api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_lines(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(revision_id): Path<Uuid>,
    Query(pq): Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<BomLine>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let all = bom_service::list_lines(&state.pool, &tenant_id, revision_id)
        .await
        .map_err(into_api_error)?;
    Ok(Json(paginate(all, &pq)))
}

// ============================================================================
// Explosion + Where-Used (tree responses — NOT paginated)
// ============================================================================

pub async fn get_explosion(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
    Query(query): Query<ExplosionQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::explode(&state.pool, &tenant_id, bom_id, &query).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => into_api_error(e).into_response(),
    }
}

pub async fn get_where_used(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Query(query): Query<WhereUsedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::where_used(&state.pool, &tenant_id, item_id, &query).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => into_api_error(e).into_response(),
    }
}
