//! HTTP handlers for party contact CRUD.
//!
//! Routes:
//!   POST   /api/party/parties/:party_id/contacts                      — create contact
//!   GET    /api/party/parties/:party_id/contacts                      — list contacts
//!   GET    /api/party/contacts/:id                                    — get contact
//!   PUT    /api/party/contacts/:id                                    — update contact
//!   DELETE /api/party/contacts/:id                                    — deactivate (soft-delete)
//!   POST   /api/party/parties/:party_id/contacts/:id/set-primary      — set primary for role
//!   GET    /api/party/parties/:party_id/primary-contacts              — primary contacts map

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::party::{extract_tenant, DataResponse};
use crate::domain::contact::{CreateContactRequest, SetPrimaryRequest, UpdateContactRequest};
use crate::domain::contact_service;
use crate::AppState;

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

/// POST /api/party/parties/:party_id/contacts
pub async fn create_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateContactRequest>,
) -> Result<(StatusCode, Json<crate::domain::contact::Contact>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let contact =
        contact_service::create_contact(&state.pool, &app_id, party_id, &req, correlation_id)
            .await
            .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(contact)))
}

/// GET /api/party/parties/:party_id/contacts
pub async fn list_contacts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<DataResponse<crate::domain::contact::Contact>>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let contacts = contact_service::list_contacts(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DataResponse { data: contacts }))
}

/// GET /api/party/contacts/:id
pub async fn get_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(contact_id): Path<Uuid>,
) -> Result<Json<crate::domain::contact::Contact>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let contact = contact_service::get_contact(&state.pool, &app_id, contact_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(format!("Contact {} not found", contact_id))
        })?;

    Ok(Json(contact))
}

/// PUT /api/party/contacts/:id
pub async fn update_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(contact_id): Path<Uuid>,
    Json(req): Json<UpdateContactRequest>,
) -> Result<Json<crate::domain::contact::Contact>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let contact =
        contact_service::update_contact(&state.pool, &app_id, contact_id, &req, correlation_id)
            .await
            .map_err(ApiError::from)?;

    Ok(Json(contact))
}

/// DELETE /api/party/contacts/:id — soft-delete (deactivate)
pub async fn delete_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(contact_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    contact_service::deactivate_contact(&state.pool, &app_id, contact_id, correlation_id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/party/parties/:party_id/contacts/:id/set-primary
pub async fn set_primary(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path((party_id, contact_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetPrimaryRequest>,
) -> Result<Json<crate::domain::contact::Contact>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    req.validate().map_err(ApiError::from)?;

    let contact = contact_service::set_primary_for_role(
        &state.pool,
        &app_id,
        party_id,
        contact_id,
        &req.role,
        correlation_id,
    )
    .await
    .map_err(ApiError::from)?;

    Ok(Json(contact))
}

/// GET /api/party/parties/:party_id/primary-contacts
pub async fn primary_contacts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<DataResponse<crate::domain::contact::PrimaryContactEntry>>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let entries = contact_service::get_primary_contacts(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DataResponse { data: entries }))
}
