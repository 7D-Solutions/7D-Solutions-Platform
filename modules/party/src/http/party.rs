//! HTTP handlers for party CRUD and search.
//!
//! Routes:
//!   POST   /api/party/companies               — create company
//!   POST   /api/party/individuals             — create individual
//!   GET    /api/party/parties/:id             — get party (with extension + refs)
//!   GET    /api/party/parties                 — list parties
//!   PUT    /api/party/parties/:id             — update base party fields
//!   POST   /api/party/parties/:id/deactivate  — soft-delete
//!   GET    /api/party/parties/search          — search by name/type/external_ref
//!
//! App identity carried in `X-App-Id` header (tenant/app scope).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::party::{
    service, CreateCompanyRequest, CreateIndividualRequest, Party, PartyError, PartyView,
    SearchQuery, UpdatePartyRequest,
};
use crate::AppState;

// ============================================================================
// Shared helpers
// ============================================================================

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

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn actor_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-actor-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string()
}

fn party_error_response(e: PartyError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        PartyError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("party_not_found", &format!("Party {} not found", id))),
        ),
        PartyError::Conflict(msg) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new("conflict", &msg)),
        ),
        PartyError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        PartyError::Database(e) => {
            tracing::error!("Party DB error: {}", e);
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
    pub fn new(error: &str, message: &str) -> Self {
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
pub struct ListPartiesQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/party/companies — create a company party
pub async fn create_company(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateCompanyRequest>,
) -> Result<(StatusCode, Json<PartyView>), (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let view = service::create_company(&state.pool, &app_id, &req, correlation_id)
        .await
        .map_err(party_error_response)?;

    Ok((StatusCode::CREATED, Json(view)))
}

/// POST /api/party/individuals — create an individual party
pub async fn create_individual(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateIndividualRequest>,
) -> Result<(StatusCode, Json<PartyView>), (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let view = service::create_individual(&state.pool, &app_id, &req, correlation_id)
        .await
        .map_err(party_error_response)?;

    Ok((StatusCode::CREATED, Json(view)))
}

/// GET /api/party/parties — list parties (base records)
pub async fn list_parties(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListPartiesQuery>,
) -> Result<Json<Vec<Party>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let parties = service::list_parties(&state.pool, &app_id, query.include_inactive)
        .await
        .map_err(party_error_response)?;

    Ok(Json(parties))
}

/// GET /api/party/parties/:id — get a single party with extension + refs
pub async fn get_party(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
) -> Result<Json<PartyView>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let view = service::get_party(&state.pool, &app_id, party_id)
        .await
        .map_err(party_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "party_not_found",
                    &format!("Party {} not found", party_id),
                )),
            )
        })?;

    Ok(Json(view))
}

/// PUT /api/party/parties/:id — update base party fields
pub async fn update_party(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
    Json(req): Json<UpdatePartyRequest>,
) -> Result<Json<PartyView>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let view = service::update_party(&state.pool, &app_id, party_id, &req, correlation_id)
        .await
        .map_err(party_error_response)?;

    Ok(Json(view))
}

/// POST /api/party/parties/:id/deactivate — soft-delete a party
pub async fn deactivate_party(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);
    let actor = actor_from_headers(&headers);

    service::deactivate_party(&state.pool, &app_id, party_id, &actor, correlation_id)
        .await
        .map_err(party_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/party/parties/search — search parties
pub async fn search_parties(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<Party>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let results = service::search_parties(&state.pool, &app_id, &query)
        .await
        .map_err(party_error_response)?;

    Ok(Json(results))
}
