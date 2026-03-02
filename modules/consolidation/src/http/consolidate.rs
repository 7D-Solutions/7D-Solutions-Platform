//! HTTP handler for consolidated trial balance.
//!
//! Tenant identity derived from JWT `VerifiedClaims`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::engine::{self, compute, EngineError};
use crate::AppState;

fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ConsolidateError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ConsolidateError {
            status: StatusCode::UNAUTHORIZED,
            message: "Missing or invalid authentication".to_string(),
        }),
    }
}

#[derive(Debug, Deserialize)]
pub struct ConsolidateQuery {
    pub period_id: Uuid,
    pub as_of: NaiveDate,
}

#[derive(Debug, Serialize)]
pub struct ConsolidateResponse {
    pub group_id: Uuid,
    pub as_of: String,
    pub reporting_currency: String,
    pub row_count: usize,
    pub rows: Vec<engine::ConsolidatedTbRow>,
    pub input_hash: String,
    pub entity_hashes: Vec<engine::EntityHashEntry>,
}

#[derive(Debug, Serialize)]
pub struct CachedTbResponse {
    pub group_id: Uuid,
    pub as_of: String,
    pub row_count: usize,
    pub rows: Vec<compute::CachedTbRow>,
    pub source: &'static str,
}

#[derive(Debug)]
pub struct ConsolidateError {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for ConsolidateError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

fn map_engine_error(e: EngineError) -> ConsolidateError {
    match &e {
        EngineError::PeriodNotClosed(_) => ConsolidateError {
            status: StatusCode::PRECONDITION_FAILED,
            message: e.to_string(),
        },
        EngineError::HashMismatch { .. } => ConsolidateError {
            status: StatusCode::CONFLICT,
            message: e.to_string(),
        },
        EngineError::MissingCoaMapping { .. } | EngineError::MissingFxPolicy(_) => {
            ConsolidateError {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: e.to_string(),
            }
        }
        EngineError::Config(_) => ConsolidateError {
            status: StatusCode::NOT_FOUND,
            message: e.to_string(),
        },
        _ => ConsolidateError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal consolidation error".to_string(),
        },
    }
}

/// POST /api/consolidation/groups/{group_id}/consolidate
///
/// Runs the full consolidation pipeline and caches the result.
pub async fn run_consolidation(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Json(params): Json<ConsolidateQuery>,
) -> Result<Json<ConsolidateResponse>, ConsolidateError> {
    let tenant_id = extract_tenant(&claims)?;
    let gl_client = app_state.gl_client();

    let result = compute::consolidate(
        &app_state.pool,
        &gl_client,
        &tenant_id,
        group_id,
        params.period_id,
        params.as_of,
    )
    .await
    .map_err(map_engine_error)?;

    app_state.metrics.consolidation_runs_total.inc();

    Ok(Json(ConsolidateResponse {
        group_id: result.group_id,
        as_of: result.as_of.to_string(),
        reporting_currency: result.reporting_currency,
        row_count: result.rows.len(),
        rows: result.rows,
        input_hash: result.input_hash,
        entity_hashes: result.entity_hashes,
    }))
}

/// GET /api/consolidation/groups/{group_id}/trial-balance?as_of=YYYY-MM-DD
///
/// Returns cached consolidated TB if available.
pub async fn get_consolidated_tb(
    State(app_state): State<Arc<AppState>>,
    Path(group_id): Path<Uuid>,
    Query(params): Query<CachedTbQuery>,
) -> Result<Json<CachedTbResponse>, ConsolidateError> {
    let cached = compute::get_cached_tb(&app_state.pool, group_id, params.as_of)
        .await
        .map_err(map_engine_error)?;

    match cached {
        Some(rows) => Ok(Json(CachedTbResponse {
            group_id,
            as_of: params.as_of.to_string(),
            row_count: rows.len(),
            rows,
            source: "cache",
        })),
        None => Err(ConsolidateError {
            status: StatusCode::NOT_FOUND,
            message: format!("No cached TB for group {} as_of {}", group_id, params.as_of),
        }),
    }
}

#[derive(Debug, Deserialize)]
pub struct CachedTbQuery {
    pub as_of: NaiveDate,
}
