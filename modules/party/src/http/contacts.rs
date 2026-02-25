//! HTTP handlers for party contact CRUD.
//!
//! Routes:
//!   POST   /api/party/parties/:party_id/contacts          — create contact
//!   GET    /api/party/parties/:party_id/contacts          — list contacts
//!   GET    /api/party/contacts/:id                        — get contact
//!   PUT    /api/party/contacts/:id                        — update contact
//!   DELETE /api/party/contacts/:id                        — delete contact

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::party::{extract_tenant, ErrorBody};
use crate::domain::contact::{CreateContactRequest, UpdateContactRequest};
use crate::domain::contact_service;
use crate::domain::party::PartyError;
use crate::AppState;

fn contact_error_response(e: PartyError) -> (StatusCode, Json<ErrorBody>) {
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
            tracing::error!("Contact DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

/// POST /api/party/parties/:party_id/contacts
pub async fn create_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
    Json(req): Json<CreateContactRequest>,
) -> Result<(StatusCode, Json<crate::domain::contact::Contact>), (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let contact = contact_service::create_contact(&state.pool, &app_id, party_id, &req)
        .await
        .map_err(contact_error_response)?;

    Ok((StatusCode::CREATED, Json(contact)))
}

/// GET /api/party/parties/:party_id/contacts
pub async fn list_contacts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<Vec<crate::domain::contact::Contact>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let contacts = contact_service::list_contacts(&state.pool, &app_id, party_id)
        .await
        .map_err(contact_error_response)?;

    Ok(Json(contacts))
}

/// GET /api/party/contacts/:id
pub async fn get_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(contact_id): Path<Uuid>,
) -> Result<Json<crate::domain::contact::Contact>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let contact = contact_service::get_contact(&state.pool, &app_id, contact_id)
        .await
        .map_err(contact_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new("not_found", &format!("Contact {} not found", contact_id))),
            )
        })?;

    Ok(Json(contact))
}

/// PUT /api/party/contacts/:id
pub async fn update_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(contact_id): Path<Uuid>,
    Json(req): Json<UpdateContactRequest>,
) -> Result<Json<crate::domain::contact::Contact>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let contact = contact_service::update_contact(&state.pool, &app_id, contact_id, &req)
        .await
        .map_err(contact_error_response)?;

    Ok(Json(contact))
}

/// DELETE /api/party/contacts/:id
pub async fn delete_contact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(contact_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    contact_service::delete_contact(&state.pool, &app_id, contact_id)
        .await
        .map_err(contact_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}
