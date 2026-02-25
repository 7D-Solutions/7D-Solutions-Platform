//! Timesheet entry HTTP handlers — create, correct, void, list, history.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::entries::{
        models::{CorrectEntryRequest, CreateEntryRequest, EntryError, VoidEntryRequest},
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
// Error mapping
// ============================================================================

fn entry_error_response(err: EntryError) -> impl IntoResponse {
    match err {
        EntryError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Entry not found" })),
        ),
        EntryError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        EntryError::PeriodLocked(msg) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "period_locked", "message": msg })),
        ),
        EntryError::Overlap => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "overlap",
                "message": "Duplicate entry for employee/date/project/task"
            })),
        ),
        EntryError::IdempotentReplay { status_code, body } => {
            let sc = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
            (sc, Json(body))
        }
        EntryError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
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

/// POST /api/timekeeping/entries
pub async fn create_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(mut req): Json<CreateEntryRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    let idem = idempotency_key(&headers);
    match service::create_entry(&state.pool, &req, idem.as_deref()).await {
        Ok(entry) => (StatusCode::CREATED, Json(json!(entry))).into_response(),
        Err(err) => entry_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/entries/correct
pub async fn correct_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(mut req): Json<CorrectEntryRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    let idem = idempotency_key(&headers);
    match service::correct_entry(&state.pool, &req, idem.as_deref()).await {
        Ok(entry) => (StatusCode::OK, Json(json!(entry))).into_response(),
        Err(err) => entry_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/entries/void
pub async fn void_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(mut req): Json<VoidEntryRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    let idem = idempotency_key(&headers);
    match service::void_entry(&state.pool, &req, idem.as_deref()).await {
        Ok(entry) => (StatusCode::OK, Json(json!(entry))).into_response(),
        Err(err) => entry_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/entries
pub async fn list_entries(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListEntriesQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_entries(&state.pool, &app_id, q.employee_id, q.from, q.to).await {
        Ok(entries) => (StatusCode::OK, Json(json!(entries))).into_response(),
        Err(err) => entry_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/entries/:entry_id/history
pub async fn entry_history(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(entry_id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::entry_history(&state.pool, &app_id, entry_id).await {
        Ok(history) => (StatusCode::OK, Json(json!(history))).into_response(),
        Err(err) => entry_error_response(err).into_response(),
    }
}
