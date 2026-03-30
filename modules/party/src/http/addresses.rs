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
use crate::domain::address::{Address, CreateAddressRequest, UpdateAddressRequest};
use crate::domain::address_service;
use crate::AppState;

#[utoipa::path(
    post,
    path = "/api/party/parties/{party_id}/addresses",
    tag = "Addresses",
    params(("party_id" = Uuid, Path, description = "Party ID")),
    request_body = CreateAddressRequest,
    responses(
        (status = 201, description = "Address created", body = Address),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateAddressRequest>,
) -> Result<(StatusCode, Json<Address>), ApiError> {
    let app_id = extract_tenant(&claims)?;

    let address = address_service::create_address(&state.pool, &app_id, party_id, &req)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(address)))
}

#[utoipa::path(
    get,
    path = "/api/party/parties/{party_id}/addresses",
    tag = "Addresses",
    params(("party_id" = Uuid, Path, description = "Party ID")),
    responses(
        (status = 200, description = "Address list", body = DataResponse<Address>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_addresses(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<DataResponse<Address>>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let addresses = address_service::list_addresses(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DataResponse { data: addresses }))
}

#[utoipa::path(
    get,
    path = "/api/party/addresses/{id}",
    tag = "Addresses",
    params(("id" = Uuid, Path, description = "Address ID")),
    responses(
        (status = 200, description = "Address details", body = Address),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(address_id): Path<Uuid>,
) -> Result<Json<Address>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let address = address_service::get_address(&state.pool, &app_id, address_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(format!("Address {} not found", address_id))
        })?;

    Ok(Json(address))
}

#[utoipa::path(
    put,
    path = "/api/party/addresses/{id}",
    tag = "Addresses",
    params(("id" = Uuid, Path, description = "Address ID")),
    request_body = UpdateAddressRequest,
    responses(
        (status = 200, description = "Address updated", body = Address),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_address(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(address_id): Path<Uuid>,
    Json(req): Json<UpdateAddressRequest>,
) -> Result<Json<Address>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let address = address_service::update_address(&state.pool, &app_id, address_id, &req)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(address))
}

#[utoipa::path(
    delete,
    path = "/api/party/addresses/{id}",
    tag = "Addresses",
    params(("id" = Uuid, Path, description = "Address ID")),
    responses(
        (status = 204, description = "Address deleted"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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
