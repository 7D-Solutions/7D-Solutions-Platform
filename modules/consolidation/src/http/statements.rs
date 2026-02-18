//! HTTP handlers for consolidated financial statement endpoints.
//!
//! Endpoints:
//!   GET /api/consolidation/groups/{group_id}/pl?as_of=YYYY-MM-DD
//!   GET /api/consolidation/groups/{group_id}/balance-sheet?as_of=YYYY-MM-DD

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::statements::{bs, pl};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct StatementQuery {
    pub as_of: NaiveDate,
}

/// GET /api/consolidation/groups/{group_id}/pl
pub async fn get_consolidated_pl(
    State(state): State<Arc<AppState>>,
    Path(group_id): Path<Uuid>,
    Query(params): Query<StatementQuery>,
) -> Result<Json<pl::ConsolidatedPl>, (StatusCode, String)> {
    pl::compute_consolidated_pl(&state.pool, group_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(group_id = %group_id, error = %e, "Consolidated P&L failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

/// GET /api/consolidation/groups/{group_id}/balance-sheet
pub async fn get_consolidated_bs(
    State(state): State<Arc<AppState>>,
    Path(group_id): Path<Uuid>,
    Query(params): Query<StatementQuery>,
) -> Result<Json<bs::ConsolidatedBalanceSheet>, (StatusCode, String)> {
    bs::compute_consolidated_bs(&state.pool, group_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(group_id = %group_id, error = %e, "Consolidated BS failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}
