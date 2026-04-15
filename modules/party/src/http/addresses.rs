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
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use platform_http_contracts::PaginatedResponse;

use super::party::with_request_id;
use crate::domain::address::{Address, CreateAddressRequest, UpdateAddressRequest};
use crate::domain::address_service;
use crate::AppState;
use platform_sdk::extract_tenant;

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
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateAddressRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match address_service::create_address(&state.pool, &app_id, party_id, &req).await {
        Ok(address) => (StatusCode::CREATED, Json(address)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/party/parties/{party_id}/addresses",
    tag = "Addresses",
    params(("party_id" = Uuid, Path, description = "Party ID")),
    responses(
        (status = 200, description = "Address list", body = PaginatedResponse<Address>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_addresses(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(party_id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match address_service::list_addresses(&state.pool, &app_id, party_id).await {
        Ok(addresses) => {
            let total = addresses.len() as i64;
            Json(PaginatedResponse::new(addresses, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
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
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(address_id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match address_service::get_address(&state.pool, &app_id, address_id).await {
        Ok(Some(address)) => Json(address).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Address {} not found", address_id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
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
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(address_id): Path<Uuid>,
    Json(req): Json<UpdateAddressRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match address_service::update_address(&state.pool, &app_id, address_id, &req).await {
        Ok(address) => Json(address).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
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
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(address_id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match address_service::delete_address(&state.pool, &app_id, address_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
