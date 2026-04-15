//! HTTP handlers for vendor CRUD — POST, GET, PUT, deactivate.
//!
//! Tenant identity is derived from JWT claims via [`VerifiedClaims`].
//! All operations are tenant-scoped; cross-tenant access is impossible.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::vendors::{service, CreateVendorRequest, UpdateVendorRequest};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Shared helpers
// ============================================================================

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListVendorsQuery {
    /// Include deactivated vendors (default: false)
    #[serde(default)]
    pub include_inactive: bool,
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post, path = "/api/ap/vendors", tag = "Vendors",
    request_body = CreateVendorRequest,
    responses((status = 201, description = "Vendor created", body = crate::domain::vendors::Vendor), (status = 401, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_vendor(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateVendorRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::create_vendor(&state.pool, &tenant_id, &req, correlation_id).await {
        Ok(vendor) => (StatusCode::CREATED, Json(vendor)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/ap/vendors", tag = "Vendors",
    responses((status = 200, description = "Vendor list", body = PaginatedResponse<crate::domain::vendors::Vendor>)),
    security(("bearer" = [])),
)]
pub async fn list_vendors(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListVendorsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::list_vendors(&state.pool, &tenant_id, query.include_inactive).await {
        Ok(vendors) => {
            let total = vendors.len() as i64;
            let resp = PaginatedResponse::new(vendors, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/ap/vendors/{vendor_id}", tag = "Vendors",
    params(("vendor_id" = Uuid, Path, description = "Vendor ID")),
    responses((status = 200, description = "Vendor details", body = crate::domain::vendors::Vendor), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_vendor(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(vendor_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::get_vendor(&state.pool, &tenant_id, vendor_id).await {
        Ok(Some(vendor)) => Json(vendor).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Vendor {} not found", vendor_id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/ap/vendors/{vendor_id}", tag = "Vendors",
    params(("vendor_id" = Uuid, Path, description = "Vendor ID")),
    request_body = UpdateVendorRequest,
    responses((status = 200, description = "Vendor updated", body = crate::domain::vendors::Vendor), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_vendor(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(vendor_id): Path<Uuid>,
    Json(req): Json<UpdateVendorRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::update_vendor(&state.pool, &tenant_id, vendor_id, &req, correlation_id).await {
        Ok(vendor) => Json(vendor).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/ap/vendors/{vendor_id}/deactivate", tag = "Vendors",
    params(("vendor_id" = Uuid, Path, description = "Vendor ID")),
    responses((status = 204, description = "Vendor deactivated"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn deactivate_vendor(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(vendor_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);
    let actor = claims
        .as_ref()
        .map(|Extension(c)| c.user_id.to_string())
        .unwrap_or_else(|| "system".to_string());

    match service::deactivate_vendor(&state.pool, &tenant_id, vendor_id, &actor, correlation_id)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
