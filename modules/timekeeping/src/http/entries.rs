//! Timesheet entry HTTP handlers — create, correct, void, list, history.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use chrono::NaiveDate;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use crate::{
    domain::entries::{
        models::{
            CorrectEntryRequest, CreateEntryRequest, EntryError, TimesheetEntry, VoidEntryRequest,
        },
        service,
    },
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListEntriesQuery {
    pub employee_id: Uuid,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

// ============================================================================
// Sub-collection wrapper
// ============================================================================

#[derive(Debug, Serialize)]
pub struct DataWrapper<T: Serialize> {
    pub data: Vec<T>,
}

// ============================================================================
// Error mapping
// ============================================================================

fn map_entry_error(err: EntryError) -> ApiError {
    match err {
        EntryError::NotFound => ApiError::not_found("Entry not found"),
        EntryError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        EntryError::PeriodLocked(msg) => ApiError::conflict(msg),
        EntryError::Overlap => {
            ApiError::conflict("Duplicate entry for employee/date/project/task")
        }
        EntryError::IdempotentReplay { status_code, body } => {
            // Preserve idempotent replay semantics — construct with original status
            let mut err = ApiError::new(status_code, "idempotent_replay", "Idempotent replay");
            // Store the original body as a detail for consumers
            if let Some(obj) = body.as_object() {
                if let Some(msg) = obj.get("message").and_then(|v| v.as_str()) {
                    err = ApiError::new(status_code, "idempotent_replay", msg);
                }
            }
            err
        }
        EntryError::Database(e) => ApiError::internal(e.to_string()),
    }
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/entries",
    request_body = CreateEntryRequest,
    responses(
        (status = 201, description = "Entry created", body = TimesheetEntry),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 409, description = "Overlap or period locked", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Entries",
)]
pub async fn create_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(mut req): Json<CreateEntryRequest>,
) -> Result<(StatusCode, Json<TimesheetEntry>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let idem = idempotency_key(&headers);
    let entry = service::create_entry(&state.pool, &req, idem.as_deref())
        .await
        .map_err(map_entry_error)?;
    Ok((StatusCode::CREATED, Json(entry)))
}

#[utoipa::path(
    post,
    path = "/api/timekeeping/entries/correct",
    request_body = CorrectEntryRequest,
    responses(
        (status = 200, description = "Entry corrected", body = TimesheetEntry),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Entry not found", body = ApiError),
        (status = 409, description = "Period locked", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Entries",
)]
pub async fn correct_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(mut req): Json<CorrectEntryRequest>,
) -> Result<Json<TimesheetEntry>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let idem = idempotency_key(&headers);
    let entry = service::correct_entry(&state.pool, &req, idem.as_deref())
        .await
        .map_err(map_entry_error)?;
    Ok(Json(entry))
}

#[utoipa::path(
    post,
    path = "/api/timekeeping/entries/void",
    request_body = VoidEntryRequest,
    responses(
        (status = 200, description = "Entry voided", body = TimesheetEntry),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Entry not found", body = ApiError),
        (status = 409, description = "Period locked", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Entries",
)]
pub async fn void_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(mut req): Json<VoidEntryRequest>,
) -> Result<Json<TimesheetEntry>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let idem = idempotency_key(&headers);
    let entry = service::void_entry(&state.pool, &req, idem.as_deref())
        .await
        .map_err(map_entry_error)?;
    Ok(Json(entry))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/entries",
    params(
        ("employee_id" = Uuid, Query, description = "Employee UUID"),
        ("from" = NaiveDate, Query, description = "Period start date"),
        ("to" = NaiveDate, Query, description = "Period end date"),
    ),
    responses(
        (status = 200, description = "Entry list", body = Vec<TimesheetEntry>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Entries",
)]
pub async fn list_entries(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListEntriesQuery>,
) -> Result<Json<PaginatedResponse<TimesheetEntry>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let entries = service::list_entries(&state.pool, &app_id, q.employee_id, q.from, q.to)
        .await
        .map_err(map_entry_error)?;
    let total = entries.len() as i64;
    Ok(Json(PaginatedResponse::new(entries, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/entries/{entry_id}/history",
    params(("entry_id" = Uuid, Path, description = "Entry UUID")),
    responses(
        (status = 200, description = "Entry version history", body = Vec<TimesheetEntry>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Entries",
)]
pub async fn entry_history(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(entry_id): Path<Uuid>,
) -> Result<Json<DataWrapper<TimesheetEntry>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let history = service::entry_history(&state.pool, &app_id, entry_id)
        .await
        .map_err(map_entry_error)?;
    Ok(Json(DataWrapper { data: history }))
}
