//! HTTP handlers for party address CRUD.
//!
//! Routes:
//!   POST   /api/party/parties/:party_id/addresses         — create address
//!   GET    /api/party/parties/:party_id/addresses         — list addresses
//!   GET    /api/party/addresses/:id                       — get address
//!   PUT    /api/party/addresses/:id                       — update address
//!   DELETE /api/party/addresses/:id                       — delete address

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::party::{extract_tenant, DataResponse};
use crate::domain::address::{CreateAddressRequest, UpdateAddressRequest};
use crate::domain::address_service;
use crate::AppState;

/// POST /api/party/parties/:party_id/addresses
pub async fn create_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateAddressRequest>,
) -> Result<(StatusCode, Json<crate::domain::address::Address>), ApiError> {
    let app_id = extract_tenant(&claims)?;

    let address = address_service::create_address(&state.pool, &app_id, party_id, &req)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(address)))
}

/// GET /api/party/parties/:party_id/addresses
pub async fn list_addresses(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<DataResponse<crate::domain::address::Address>>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let addresses = address_service::list_addresses(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DataResponse { data: addresses }))
}

/// GET /api/party/addresses/:id
pub async fn get_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(address_id): Path<Uuid>,
) -> Result<Json<crate::domain::address::Address>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let address = address_service::get_address(&state.pool, &app_id, address_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(format!("Address {} not found", address_id))
        })?;

    Ok(Json(address))
}

/// PUT /api/party/addresses/:id
pub async fn update_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(address_id): Path<Uuid>,
    Json(req): Json<UpdateAddressRequest>,
) -> Result<Json<crate::domain::address::Address>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let address = address_service::update_address(&state.pool, &app_id, address_id, &req)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(address))
}

/// DELETE /api/party/addresses/:id
pub async fn delete_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(address_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let app_id = extract_tenant(&claims)?;

    address_service::delete_address(&state.pool, &app_id, address_id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}
