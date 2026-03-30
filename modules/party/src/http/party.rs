//! HTTP handlers for party CRUD and search.
//!
//! Routes:
//!   POST   /api/party/companies               — create company
//!   POST   /api/party/individuals             — create individual
//!   GET    /api/party/parties/:id             — get party (with extension + refs)
//!   GET    /api/party/parties                 — list parties (paginated)
//!   PUT    /api/party/parties/:id             — update base party fields
//!   POST   /api/party/parties/:id/deactivate  — soft-delete
//!   GET    /api/party/parties/search          — search by name/type/external_ref (paginated)
//!
//! App identity derived from JWT `VerifiedClaims` (tenant/app scope).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::domain::party::{
    service, CreateCompanyRequest, CreateIndividualRequest, Party, PartyView, SearchQuery,
    UpdatePartyRequest,
};
use crate::AppState;

// ============================================================================
// Shared helpers
// ============================================================================

pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized(
            "Missing or invalid authentication",
        )),
    }
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

/// Simple wrapper for sub-collection list endpoints that don't need
/// full pagination (contacts per party, addresses per party, etc.).
#[derive(Debug, Serialize, ToSchema)]
pub struct DataResponse<T: Serialize + ToSchema> {
    pub data: Vec<T>,
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListPartiesQuery {
    #[serde(default)]
    pub include_inactive: bool,
    /// 1-based page number (default: 1)
    pub page: Option<i64>,
    /// Items per page, 1-200 (default: 50)
    pub page_size: Option<i64>,
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/party/companies",
    tag = "Parties",
    request_body = CreateCompanyRequest,
    responses(
        (status = 201, description = "Company party created", body = PartyView),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_company(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateCompanyRequest>,
) -> Result<(StatusCode, Json<PartyView>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let view = service::create_company(&state.pool, &app_id, &req, correlation_id)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(view)))
}

#[utoipa::path(
    post,
    path = "/api/party/individuals",
    tag = "Parties",
    request_body = CreateIndividualRequest,
    responses(
        (status = 201, description = "Individual party created", body = PartyView),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_individual(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateIndividualRequest>,
) -> Result<(StatusCode, Json<PartyView>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let view = service::create_individual(&state.pool, &app_id, &req, correlation_id)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(view)))
}

#[utoipa::path(
    get,
    path = "/api/party/parties",
    tag = "Parties",
    params(ListPartiesQuery),
    responses(
        (status = 200, description = "Paginated party list", body = PaginatedResponse<Party>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_parties(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListPartiesQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);

    match service::list_parties(&state.pool, &app_id, query.include_inactive, page, page_size)
        .await
    {
        Ok((parties, total)) => {
            let resp = PaginatedResponse::new(parties, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            api_err.into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/party/parties/{id}",
    tag = "Parties",
    params(("id" = Uuid, Path, description = "Party ID")),
    responses(
        (status = 200, description = "Party details with extensions", body = PartyView),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_party(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(party_id): Path<Uuid>,
) -> Result<Json<PartyView>, ApiError> {
    let app_id = extract_tenant(&claims)?;

    let view = service::get_party(&state.pool, &app_id, party_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(format!("Party {} not found", party_id))
        })?;

    Ok(Json(view))
}

#[utoipa::path(
    put,
    path = "/api/party/parties/{id}",
    tag = "Parties",
    params(("id" = Uuid, Path, description = "Party ID")),
    request_body = UpdatePartyRequest,
    responses(
        (status = 200, description = "Party updated", body = PartyView),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_party(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
    Json(req): Json<UpdatePartyRequest>,
) -> Result<Json<PartyView>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let view = service::update_party(&state.pool, &app_id, party_id, &req, correlation_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(view))
}

#[utoipa::path(
    post,
    path = "/api/party/parties/{id}/deactivate",
    tag = "Parties",
    params(("id" = Uuid, Path, description = "Party ID")),
    responses(
        (status = 204, description = "Party deactivated"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn deactivate_party(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(party_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);
    let actor = actor_from_headers(&headers);

    service::deactivate_party(&state.pool, &app_id, party_id, &actor, correlation_id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/party/parties/search",
    tag = "Parties",
    params(SearchQuery),
    responses(
        (status = 200, description = "Paginated search results", body = PaginatedResponse<Party>),
    ),
    security(("bearer" = [])),
)]
pub async fn search_parties(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let page_size = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    let page = (offset / page_size) + 1;

    match service::search_parties(&state.pool, &app_id, &query).await {
        Ok((results, total)) => {
            let resp = PaginatedResponse::new(results, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            api_err.into_response()
        }
    }
}
