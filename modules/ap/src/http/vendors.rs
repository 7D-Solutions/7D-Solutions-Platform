//! HTTP handlers for vendor CRUD — POST, GET, PUT, deactivate.
//!
//! Tenant identity is carried in the `X-Tenant-Id` header.
//! All operations are tenant-scoped; cross-tenant access is impossible.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::vendors::{
    service, CreateVendorRequest, UpdateVendorRequest, Vendor, VendorError,
};
use crate::AppState;

// ============================================================================
// Shared helpers
// ============================================================================

fn tenant_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_tenant", "X-Tenant-Id header is required")),
            )
        })
}

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn vendor_error_response(e: VendorError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        VendorError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("vendor_not_found", &format!("Vendor {} not found", id))),
        ),
        VendorError::DuplicateName(name) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_vendor_name",
                &format!("Active vendor '{}' already exists for this tenant", name),
            )),
        ),
        VendorError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        VendorError::Database(e) => {
            tracing::error!("AP vendors DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
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
    headers: HeaderMap,
    Json(req): Json<CreateVendorRequest>,
) -> Result<(StatusCode, Json<Vendor>), (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let vendor = service::create_vendor(&state.pool, &tenant_id, &req, correlation_id)
        .await
        .map_err(vendor_error_response)?;

    Ok((StatusCode::CREATED, Json(vendor)))
}

/// GET /api/ap/vendors — list vendors for tenant
pub async fn list_vendors(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListVendorsQuery>,
) -> Result<Json<Vec<Vendor>>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let vendors =
        service::list_vendors(&state.pool, &tenant_id, query.include_inactive)
            .await
            .map_err(vendor_error_response)?;

    Ok(Json(vendors))
}

/// GET /api/ap/vendors/:vendor_id — get a single vendor
pub async fn get_vendor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(vendor_id): Path<Uuid>,
) -> Result<Json<Vendor>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let vendor = service::get_vendor(&state.pool, &tenant_id, vendor_id)
        .await
        .map_err(vendor_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "vendor_not_found",
                    &format!("Vendor {} not found", vendor_id),
                )),
            )
        })?;

    Ok(Json(vendor))
}

/// PUT /api/ap/vendors/:vendor_id — update vendor fields
pub async fn update_vendor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(vendor_id): Path<Uuid>,
    Json(req): Json<UpdateVendorRequest>,
) -> Result<Json<Vendor>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let vendor =
        service::update_vendor(&state.pool, &tenant_id, vendor_id, &req, correlation_id)
            .await
            .map_err(vendor_error_response)?;

    Ok(Json(vendor))
}

/// POST /api/ap/vendors/:vendor_id/deactivate — soft-delete a vendor
pub async fn deactivate_vendor(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(vendor_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);
    let actor = headers
        .get("x-actor-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string();

    service::deactivate_vendor(&state.pool, &tenant_id, vendor_id, &actor, correlation_id)
        .await
        .map_err(vendor_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}
