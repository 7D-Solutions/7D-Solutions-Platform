//! HTTP handlers for consolidated financial statement endpoints.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::domain::statements::{bs, pl};
use crate::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatementQuery {
    pub as_of: NaiveDate,
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/pl", tag = "Statements",
    params(("group_id" = Uuid, Path), StatementQuery),
    responses((status = 200, body = pl::ConsolidatedPl), (status = 500, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_consolidated_pl(
    State(state): State<Arc<AppState>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Query(params): Query<StatementQuery>,
) -> impl IntoResponse {
    match pl::compute_consolidated_pl(&state.pool, group_id, params.as_of).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => {
            tracing::error!(group_id = %group_id, error = %e, "Consolidated P&L failed");
            with_request_id(
                ApiError::internal("Consolidated P&L computation failed"),
                &ctx,
            )
            .into_response()
        }
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/balance-sheet", tag = "Statements",
    params(("group_id" = Uuid, Path), StatementQuery),
    responses((status = 200, body = bs::ConsolidatedBalanceSheet), (status = 500, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_consolidated_bs(
    State(state): State<Arc<AppState>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Query(params): Query<StatementQuery>,
) -> impl IntoResponse {
    match bs::compute_consolidated_bs(&state.pool, group_id, params.as_of).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => {
            tracing::error!(group_id = %group_id, error = %e, "Consolidated BS failed");
            with_request_id(
                ApiError::internal("Consolidated balance sheet computation failed"),
                &ctx,
            )
            .into_response()
        }
    }
}
