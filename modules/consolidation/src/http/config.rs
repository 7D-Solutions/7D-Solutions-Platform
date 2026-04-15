//! HTTP handlers for consolidation config CRUD.
//!
//! Tenant identity derived from JWT `VerifiedClaims` (maps to tenant_id).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::config::{self, models::*, service, service_rules};
use crate::AppState;

// ============================================================================
// Groups
// ============================================================================

#[utoipa::path(
    post, path = "/api/consolidation/groups", tag = "Groups",
    request_body = CreateGroupRequest,
    responses((status = 201, body = config::Group), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateGroupRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::create_group(&state.pool, &tid, &req).await {
        Ok(g) => (StatusCode::CREATED, Json(g)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups", tag = "Groups",
    params(ListGroupsQuery),
    responses((status = 200, body = PaginatedResponse<config::Group>)),
    security(("bearer" = [])),
)]
pub async fn list_groups(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(q): Query<ListGroupsQuery>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::list_groups(&state.pool, &tid, q.include_inactive).await {
        Ok(rows) => {
            let total = rows.len() as i64;
            Json(PaginatedResponse::new(rows, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{id}", tag = "Groups",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = config::Group), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::get_group(&state.pool, &tid, id).await {
        Ok(g) => Json(g).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/consolidation/groups/{id}", tag = "Groups",
    params(("id" = Uuid, Path)),
    request_body = UpdateGroupRequest,
    responses((status = 200, body = config::Group), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateGroupRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::update_group(&state.pool, &tid, id, &req).await {
        Ok(g) => Json(g).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/consolidation/groups/{id}", tag = "Groups",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn delete_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::delete_group(&state.pool, &tid, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{id}/validate", tag = "Groups",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ValidationResult)),
    security(("bearer" = [])),
)]
pub async fn validate_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::validate_group_completeness(&state.pool, &tid, group_id).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

// ============================================================================
// Entities
// ============================================================================

#[utoipa::path(
    post, path = "/api/consolidation/groups/{group_id}/entities", tag = "Entities",
    params(("group_id" = Uuid, Path)),
    request_body = CreateEntityRequest,
    responses((status = 201, body = config::GroupEntity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateEntityRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::create_entity(&state.pool, &tid, group_id, &req).await {
        Ok(entity) => (StatusCode::CREATED, Json(entity)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/entities", tag = "Entities",
    params(("group_id" = Uuid, Path), ListEntitiesQuery),
    responses((status = 200, body = PaginatedResponse<config::GroupEntity>)),
    security(("bearer" = [])),
)]
pub async fn list_entities(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Query(q): Query<ListEntitiesQuery>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::list_entities(&state.pool, &tid, group_id, q.include_inactive).await {
        Ok(rows) => {
            let total = rows.len() as i64;
            Json(PaginatedResponse::new(rows, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/entities/{id}", tag = "Entities",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = config::GroupEntity), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::get_entity(&state.pool, &tid, id).await {
        Ok(e) => Json(e).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/consolidation/entities/{id}", tag = "Entities",
    params(("id" = Uuid, Path)),
    request_body = UpdateEntityRequest,
    responses((status = 200, body = config::GroupEntity), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEntityRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::update_entity(&state.pool, &tid, id, &req).await {
        Ok(entity) => Json(entity).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/consolidation/entities/{id}", tag = "Entities",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn delete_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::delete_entity(&state.pool, &tid, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

// ============================================================================
// COA mappings
// ============================================================================

#[utoipa::path(
    post, path = "/api/consolidation/groups/{group_id}/coa-mappings", tag = "COA Mappings",
    params(("group_id" = Uuid, Path)),
    request_body = CreateCoaMappingRequest,
    responses((status = 201, body = config::CoaMapping), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_coa_mapping(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateCoaMappingRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::create_coa_mapping(&state.pool, &tid, group_id, &req).await {
        Ok(m) => (StatusCode::CREATED, Json(m)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/coa-mappings", tag = "COA Mappings",
    params(("group_id" = Uuid, Path), ListCoaMappingsQuery),
    responses((status = 200, body = PaginatedResponse<config::CoaMapping>)),
    security(("bearer" = [])),
)]
pub async fn list_coa_mappings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Query(q): Query<ListCoaMappingsQuery>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::list_coa_mappings(&state.pool, &tid, group_id, q.entity_tenant_id.as_deref())
        .await
    {
        Ok(rows) => {
            let total = rows.len() as i64;
            Json(PaginatedResponse::new(rows, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/consolidation/coa-mappings/{id}", tag = "COA Mappings",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn delete_coa_mapping(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service::delete_coa_mapping(&state.pool, &tid, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

// ============================================================================
// Elimination rules
// ============================================================================

#[utoipa::path(
    post, path = "/api/consolidation/groups/{group_id}/elimination-rules", tag = "Elimination Rules",
    params(("group_id" = Uuid, Path)),
    request_body = CreateEliminationRuleRequest,
    responses((status = 201, body = config::EliminationRule), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateEliminationRuleRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::create_elimination_rule(&state.pool, &tid, group_id, &req).await {
        Ok(r) => (StatusCode::CREATED, Json(r)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/elimination-rules", tag = "Elimination Rules",
    params(("group_id" = Uuid, Path), ListEliminationRulesQuery),
    responses((status = 200, body = PaginatedResponse<config::EliminationRule>)),
    security(("bearer" = [])),
)]
pub async fn list_elimination_rules(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Query(q): Query<ListEliminationRulesQuery>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::list_elimination_rules(&state.pool, &tid, group_id, q.include_inactive)
        .await
    {
        Ok(rows) => {
            let total = rows.len() as i64;
            Json(PaginatedResponse::new(rows, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/elimination-rules/{id}", tag = "Elimination Rules",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = config::EliminationRule), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::get_elimination_rule(&state.pool, &tid, id).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/consolidation/elimination-rules/{id}", tag = "Elimination Rules",
    params(("id" = Uuid, Path)),
    request_body = UpdateEliminationRuleRequest,
    responses((status = 200, body = config::EliminationRule), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEliminationRuleRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::update_elimination_rule(&state.pool, &tid, id, &req).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/consolidation/elimination-rules/{id}", tag = "Elimination Rules",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn delete_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::delete_elimination_rule(&state.pool, &tid, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

// ============================================================================
// FX policies
// ============================================================================

#[utoipa::path(
    put, path = "/api/consolidation/groups/{group_id}/fx-policies", tag = "FX Policies",
    params(("group_id" = Uuid, Path)),
    request_body = UpsertFxPolicyRequest,
    responses((status = 200, body = config::FxPolicy), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn upsert_fx_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<UpsertFxPolicyRequest>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::upsert_fx_policy(&state.pool, &tid, group_id, &req).await {
        Ok(p) => Json(p).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/fx-policies", tag = "FX Policies",
    params(("group_id" = Uuid, Path)),
    responses((status = 200, body = PaginatedResponse<config::FxPolicy>)),
    security(("bearer" = [])),
)]
pub async fn list_fx_policies(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::list_fx_policies(&state.pool, &tid, group_id).await {
        Ok(rows) => {
            let total = rows.len() as i64;
            Json(PaginatedResponse::new(rows, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/consolidation/fx-policies/{id}", tag = "FX Policies",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn delete_fx_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tid = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match service_rules::delete_fx_policy(&state.pool, &tid, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
