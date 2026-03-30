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
use crate::domain::contact::{
    Contact, CreateContactRequest, PrimaryContactEntry, SetPrimaryRequest, UpdateContactRequest,
};
use crate::domain::contact_service;
use crate::AppState;

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

#[utoipa::path(
    post,
    path = "/api/party/parties/{party_id}/contacts",
    tag = "Contacts",
    params(("party_id" = Uuid, Path, description = "Party ID")),
    request_body = CreateContactRequest,
    responses(
        (status = 201, description = "Contact created", body = Contact),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateContactRequest>,
) -> Result<(StatusCode, Json<Contact>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let contact =
        contact_service::create_contact(&state.pool, &app_id, party_id, &req, correlation_id)
            .await
            .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(contact)))
}

#[utoipa::path(
    get,
    path = "/api/party/parties/{party_id}/contacts",
    tag = "Contacts",
    params(("party_id" = Uuid, Path, description = "Party ID")),
    responses(
        (status = 200, description = "Contact list", body = DataResponse<Contact>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_contacts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<DataResponse<Contact>>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let contacts = contact_service::list_contacts(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DataResponse { data: contacts }))
}

#[utoipa::path(
    get,
    path = "/api/party/contacts/{id}",
    tag = "Contacts",
    params(("id" = Uuid, Path, description = "Contact ID")),
    responses(
        (status = 200, description = "Contact details", body = Contact),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(contact_id): Path<Uuid>,
) -> Result<Json<Contact>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let contact = contact_service::get_contact(&state.pool, &app_id, contact_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(format!("Contact {} not found", contact_id))
        })?;

    Ok(Json(contact))
}

#[utoipa::path(
    put,
    path = "/api/party/contacts/{id}",
    tag = "Contacts",
    params(("id" = Uuid, Path, description = "Contact ID")),
    request_body = UpdateContactRequest,
    responses(
        (status = 200, description = "Contact updated", body = Contact),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(contact_id): Path<Uuid>,
    Json(req): Json<UpdateContactRequest>,
) -> Result<Json<Contact>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let contact =
        contact_service::update_contact(&state.pool, &app_id, contact_id, &req, correlation_id)
            .await
            .map_err(ApiError::from)?;

    Ok(Json(contact))
}

#[utoipa::path(
    delete,
    path = "/api/party/contacts/{id}",
    tag = "Contacts",
    params(("id" = Uuid, Path, description = "Contact ID")),
    responses(
        (status = 204, description = "Contact deactivated"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    post,
    path = "/api/party/parties/{party_id}/contacts/{id}/set-primary",
    tag = "Contacts",
    params(
        ("party_id" = Uuid, Path, description = "Party ID"),
        ("id" = Uuid, Path, description = "Contact ID"),
    ),
    request_body = SetPrimaryRequest,
    responses(
        (status = 200, description = "Contact set as primary for role", body = Contact),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn set_primary(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path((party_id, contact_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetPrimaryRequest>,
) -> Result<Json<Contact>, ApiError> {
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

#[utoipa::path(
    get,
    path = "/api/party/parties/{party_id}/primary-contacts",
    tag = "Contacts",
    params(("party_id" = Uuid, Path, description = "Party ID")),
    responses(
        (status = 200, description = "Primary contacts by role", body = DataResponse<PrimaryContactEntry>),
    ),
    security(("bearer" = [])),
)]
pub async fn primary_contacts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<DataResponse<PrimaryContactEntry>>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let entries = contact_service::get_primary_contacts(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DataResponse { data: entries }))
}
