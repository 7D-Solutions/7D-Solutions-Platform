//! Location HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/locations                  — create location
//!   GET   /api/inventory/locations/:id              — get location
//!   PUT   /api/inventory/locations/:id              — update location
//!   POST  /api/inventory/locations/:id/deactivate   — soft-delete location
//!   GET   /api/inventory/warehouses/:wid/locations  — list by warehouse
//!
//! Tenant identity derived from JWT `VerifiedClaims`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::locations::{CreateLocationRequest, Location, LocationRepo, UpdateLocationRequest},
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListLocationsQuery {
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
    path = "/api/inventory/locations",
    tag = "Locations",
    request_body = CreateLocationRequest,
    responses(
        (status = 201, description = "Location created", body = Location),
        (status = 409, description = "Duplicate location code", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_location(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateLocationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match LocationRepo::create(&state.pool, &req).await {
        Ok(loc) => (StatusCode::CREATED, Json(loc)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/locations/{id}",
    tag = "Locations",
    params(("id" = Uuid, Path, description = "Location ID")),
    responses(
        (status = 200, description = "Location details", body = Location),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_location(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match LocationRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(loc)) => (StatusCode::OK, Json(loc)).into_response(),
        Ok(None) => {
            with_request_id(ApiError::not_found("Location not found"), &tracing_ctx).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    put,
    path = "/api/inventory/locations/{id}",
    tag = "Locations",
    params(("id" = Uuid, Path, description = "Location ID")),
    request_body = UpdateLocationRequest,
    responses(
        (status = 200, description = "Location updated", body = Location),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_location(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateLocationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match LocationRepo::update(&state.pool, id, &req).await {
        Ok(loc) => (StatusCode::OK, Json(loc)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/locations/{id}/deactivate",
    tag = "Locations",
    params(("id" = Uuid, Path, description = "Location ID")),
    responses(
        (status = 200, description = "Location deactivated (idempotent)", body = Location),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn deactivate_location(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match LocationRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(loc) => (StatusCode::OK, Json(loc)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/warehouses/{warehouse_id}/locations",
    tag = "Locations",
    params(("warehouse_id" = Uuid, Path, description = "Warehouse ID")),
    responses(
        (status = 200, description = "Paginated location list", body = PaginatedResponse<Location>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_locations(
    State(state): State<Arc<AppState>>,
    Path(warehouse_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListLocationsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match LocationRepo::list_for_warehouse(&state.pool, &tenant_id, warehouse_id).await {
        Ok(all_locs) => {
            let total = all_locs.len() as i64;
            let page_size = q.page_size.clamp(1, 200);
            let page = q.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let locs: Vec<_> = all_locs
                .into_iter()
                .skip(offset)
                .take(page_size as usize)
                .collect();
            let resp = PaginatedResponse::new(locs, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_location_routes_compile() {}
}
