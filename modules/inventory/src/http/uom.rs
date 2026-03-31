//! UoM HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/uoms                             — create UoM
//!   GET  /api/inventory/uoms                             — list UoMs for tenant
//!   POST /api/inventory/items/:id/uom-conversions       — add conversion
//!   GET  /api/inventory/items/:id/uom-conversions       — list conversions
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
use serde_json::json;
use std::sync::Arc;
use utoipa::IntoParams;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::uom::models::{
        ConversionRepo, CreateConversionRequest, CreateUomRequest, ItemUomConversion, Uom, UomRepo,
    },
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, IntoParams)]
pub struct PaginationQuery {
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
    path = "/api/inventory/uoms",
    tag = "UoM",
    request_body = CreateUomRequest,
    responses(
        (status = 201, description = "UoM created", body = Uom),
        (status = 409, description = "Duplicate UoM code", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_uom(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateUomRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match UomRepo::create(&state.pool, &req).await {
        Ok(uom) => (StatusCode::CREATED, Json(json!(uom))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/uoms",
    tag = "UoM",
    params(PaginationQuery),
    responses(
        (status = 200, description = "Paginated UoM list", body = PaginatedResponse<Uom>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_uoms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(pq): Query<PaginationQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match UomRepo::list_for_tenant(&state.pool, &tenant_id).await {
        Ok(all_uoms) => {
            let total = all_uoms.len() as i64;
            let page_size = pq.page_size.clamp(1, 200);
            let page = pq.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let uoms: Vec<_> = all_uoms.into_iter().skip(offset).take(page_size as usize).collect();
            let resp = PaginatedResponse::new(uoms, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/items/{id}/uom-conversions",
    tag = "UoM",
    params(("id" = Uuid, Path, description = "Item ID")),
    request_body = CreateConversionRequest,
    responses(
        (status = 201, description = "Conversion created", body = ItemUomConversion),
        (status = 409, description = "Duplicate conversion", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_conversion(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateConversionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match ConversionRepo::create(&state.pool, item_id, &req).await {
        Ok(conv) => (StatusCode::CREATED, Json(json!(conv))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{id}/uom-conversions",
    tag = "UoM",
    params(("id" = Uuid, Path, description = "Item ID"), PaginationQuery),
    responses(
        (status = 200, description = "Paginated conversion list", body = PaginatedResponse<ItemUomConversion>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_conversions(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(pq): Query<PaginationQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match ConversionRepo::list_for_item(&state.pool, item_id, &tenant_id).await {
        Ok(all_convs) => {
            let total = all_convs.len() as i64;
            let page_size = pq.page_size.clamp(1, 200);
            let page = pq.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let convs: Vec<_> = all_convs.into_iter().skip(offset).take(page_size as usize).collect();
            let resp = PaginatedResponse::new(convs, page, page_size, total);
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
    fn placeholder_uom_routes_compile() {}
}
