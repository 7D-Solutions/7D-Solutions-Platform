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
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use super::party::ErrorBody;
use crate::domain::address::{CreateAddressRequest, UpdateAddressRequest};
use crate::domain::address_service;
use crate::domain::party::PartyError;
use crate::AppState;

fn app_id_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_app_id", "X-App-Id header is required")),
            )
        })
}

fn address_error_response(e: PartyError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        PartyError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", &format!("Resource {} not found", id))),
        ),
        PartyError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        PartyError::Conflict(msg) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new("conflict", &msg)),
        ),
        PartyError::Database(e) => {
            tracing::error!("Address DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

/// POST /api/party/parties/:party_id/addresses
pub async fn create_address(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateAddressRequest>,
) -> Result<(StatusCode, Json<crate::domain::address::Address>), (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let address = address_service::create_address(&state.pool, &app_id, party_id, &req)
        .await
        .map_err(address_error_response)?;

    Ok((StatusCode::CREATED, Json(address)))
}

/// GET /api/party/parties/:party_id/addresses
pub async fn list_addresses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
) -> Result<Json<Vec<crate::domain::address::Address>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let addresses = address_service::list_addresses(&state.pool, &app_id, party_id)
        .await
        .map_err(address_error_response)?;

    Ok(Json(addresses))
}

/// GET /api/party/addresses/:id
pub async fn get_address(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(address_id): Path<Uuid>,
) -> Result<Json<crate::domain::address::Address>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let address = address_service::get_address(&state.pool, &app_id, address_id)
        .await
        .map_err(address_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new("not_found", &format!("Address {} not found", address_id))),
            )
        })?;

    Ok(Json(address))
}

/// PUT /api/party/addresses/:id
pub async fn update_address(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(address_id): Path<Uuid>,
    Json(req): Json<UpdateAddressRequest>,
) -> Result<Json<crate::domain::address::Address>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let address = address_service::update_address(&state.pool, &app_id, address_id, &req)
        .await
        .map_err(address_error_response)?;

    Ok(Json(address))
}

/// DELETE /api/party/addresses/:id
pub async fn delete_address(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(address_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    address_service::delete_address(&state.pool, &app_id, address_id)
        .await
        .map_err(address_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}
