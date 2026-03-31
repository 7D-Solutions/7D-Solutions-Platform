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
use crate::http::tenant::{extract_tenant, with_request_id};
use crate::AppState;

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

/// POST /api/ap/vendors — create a vendor
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

/// GET /api/ap/vendors — list vendors for tenant
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

/// GET /api/ap/vendors/:vendor_id — get a single vendor
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

/// PUT /api/ap/vendors/:vendor_id — update vendor fields
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

/// POST /api/ap/vendors/:vendor_id/deactivate — soft-delete a vendor
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
