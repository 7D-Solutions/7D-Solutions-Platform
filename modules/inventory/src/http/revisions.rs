//! Item revision HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items/:item_id/revisions                         — create revision
//!   POST  /api/inventory/items/:item_id/revisions/:revision_id/activate   — activate revision
//!   PUT   /api/inventory/items/:item_id/revisions/:revision_id/policy-flags — update draft policy flags
//!   GET   /api/inventory/items/:item_id/revisions/at                      — query at time T
//!   GET   /api/inventory/items/:item_id/revisions                         — list all revisions

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::revisions::{
        activate_revision, create_revision, list_revisions, revision_at, update_revision_policy,
        ActivateRevisionRequest, CreateRevisionRequest, ItemRevision, UpdateRevisionPolicyRequest,
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RevisionAtQuery {
    pub t: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ListRevisionsQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/items/{item_id}/revisions",
    tag = "Revisions",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    request_body = CreateRevisionRequest,
    responses(
        (status = 201, description = "Revision created", body = ItemRevision),
        (status = 200, description = "Idempotency replay", body = ItemRevision),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_create_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    req.item_id = item_id;

    match create_revision(&state.pool, &req).await {
        Ok((rev, false)) => (StatusCode::CREATED, Json(rev)).into_response(),
        Ok((rev, true)) => (StatusCode::OK, Json(rev)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/items/{item_id}/revisions/{revision_id}/activate",
    tag = "Revisions",
    params(
        ("item_id" = Uuid, Path, description = "Item ID"),
        ("revision_id" = Uuid, Path, description = "Revision ID"),
    ),
    request_body = ActivateRevisionRequest,
    responses(
        (status = 200, description = "Revision activated", body = ItemRevision),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
        (status = 422, description = "Already activated or window overlap", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_activate_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((item_id, revision_id)): Path<(Uuid, Uuid)>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<ActivateRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match activate_revision(&state.pool, item_id, revision_id, &req).await {
        Ok((rev, _)) => (StatusCode::OK, Json(rev)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    put,
    path = "/api/inventory/items/{item_id}/revisions/{revision_id}/policy-flags",
    tag = "Revisions",
    params(
        ("item_id" = Uuid, Path, description = "Item ID"),
        ("revision_id" = Uuid, Path, description = "Revision ID"),
    ),
    request_body = UpdateRevisionPolicyRequest,
    responses(
        (status = 200, description = "Policy flags updated", body = ItemRevision),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
        (status = 422, description = "Only draft revisions can be updated", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn put_revision_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((item_id, revision_id)): Path<(Uuid, Uuid)>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateRevisionPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match update_revision_policy(&state.pool, item_id, revision_id, &req).await {
        Ok((rev, _)) => (StatusCode::OK, Json(rev)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/revisions/at",
    tag = "Revisions",
    params(
        ("item_id" = Uuid, Path, description = "Item ID"),
        ("t" = Option<String>, Query, description = "ISO 8601 timestamp (defaults to now)"),
    ),
    responses(
        (status = 200, description = "Effective revision at requested time", body = ItemRevision),
        (status = 404, description = "No revision effective at requested time", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_revision_at(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Query(query): Query<RevisionAtQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let at = query.t.unwrap_or_else(Utc::now);

    match revision_at(&state.pool, &tenant_id, item_id, at).await {
        Ok(Some(rev)) => (StatusCode::OK, Json(rev)).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found("No revision effective at requested time"),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/revisions",
    tag = "Revisions",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Paginated revision list", body = PaginatedResponse<ItemRevision>),
    ),
    security(("bearer" = [])),
)]
pub async fn get_list_revisions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Query(q): Query<ListRevisionsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match list_revisions(&state.pool, &tenant_id, item_id).await {
        Ok(all_revs) => {
            let total = all_revs.len() as i64;
            let page_size = q.page_size.clamp(1, 200);
            let page = q.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let revs: Vec<_> = all_revs
                .into_iter()
                .skip(offset)
                .take(page_size as usize)
                .collect();
            let resp = PaginatedResponse::new(revs, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}
