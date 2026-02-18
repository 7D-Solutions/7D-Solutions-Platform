//! HTTP handlers for intercompany matching and elimination posting.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::eliminations::{self, service as elim_service};
use crate::domain::intercompany::{self, service as ic_service};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct IntercompanyMatchRequest {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub as_of: NaiveDate,
}

#[derive(Debug, Serialize)]
pub struct IntercompanyMatchResponse {
    pub group_id: Uuid,
    pub as_of: String,
    pub match_count: usize,
    pub unmatched_count: usize,
    pub total_matched_minor: i64,
    pub matches: Vec<intercompany::IntercompanyMatch>,
    pub suggestions: Vec<eliminations::EliminationSuggestion>,
}

#[derive(Debug, Deserialize)]
pub struct PostEliminationsRequest {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub as_of: NaiveDate,
    pub reporting_currency: String,
}

#[derive(Debug, Serialize)]
pub struct PostEliminationsResponse {
    pub group_id: Uuid,
    pub period_id: Uuid,
    pub posted_count: usize,
    pub idempotency_key: String,
    pub journal_entry_ids: Vec<Uuid>,
    pub already_posted: bool,
}

#[derive(Debug)]
pub struct IntercompanyError {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for IntercompanyError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

fn map_error(e: crate::domain::engine::EngineError) -> IntercompanyError {
    IntercompanyError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: e.to_string(),
    }
}

/// POST /api/consolidation/groups/{group_id}/intercompany-match
///
/// Run intercompany matching and return suggestions.
pub async fn run_intercompany_match(
    State(app_state): State<Arc<AppState>>,
    Path(group_id): Path<Uuid>,
    Json(params): Json<IntercompanyMatchRequest>,
) -> Result<Json<IntercompanyMatchResponse>, IntercompanyError> {
    let gl_client = app_state.gl_client();

    let match_result = ic_service::match_intercompany_for_group(
        &app_state.pool,
        &gl_client,
        &params.tenant_id,
        group_id,
        params.period_id,
        params.as_of,
    )
    .await
    .map_err(map_error)?;

    let suggestions = elim_service::suggest_eliminations(&match_result);

    Ok(Json(IntercompanyMatchResponse {
        group_id,
        as_of: params.as_of.to_string(),
        match_count: match_result.matches.len(),
        unmatched_count: match_result.unmatched_count,
        total_matched_minor: match_result.total_matched_minor,
        matches: match_result.matches,
        suggestions,
    }))
}

/// POST /api/consolidation/groups/{group_id}/eliminations
///
/// Post elimination journals to GL. Idempotent per group+period.
pub async fn post_eliminations(
    State(app_state): State<Arc<AppState>>,
    Path(group_id): Path<Uuid>,
    Json(params): Json<PostEliminationsRequest>,
) -> Result<Json<PostEliminationsResponse>, IntercompanyError> {
    let gl_client = app_state.gl_client();

    // First, run matching to get current suggestions
    let match_result = ic_service::match_intercompany_for_group(
        &app_state.pool,
        &gl_client,
        &params.tenant_id,
        group_id,
        params.period_id,
        params.as_of,
    )
    .await
    .map_err(map_error)?;

    let suggestions = elim_service::suggest_eliminations(&match_result);

    // Post to GL
    let post_result = elim_service::post_eliminations(
        &app_state.pool,
        &gl_client,
        &params.tenant_id,
        group_id,
        params.period_id,
        params.as_of,
        &suggestions,
        &params.reporting_currency,
    )
    .await
    .map_err(map_error)?;

    Ok(Json(PostEliminationsResponse {
        group_id,
        period_id: params.period_id,
        posted_count: post_result.posted_count,
        idempotency_key: post_result.idempotency_key,
        journal_entry_ids: post_result.journal_entry_ids,
        already_posted: post_result.already_posted,
    }))
}
