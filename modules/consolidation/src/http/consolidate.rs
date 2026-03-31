//! HTTP handler for consolidated trial balance.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::domain::engine::{self, compute};
use crate::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConsolidateQuery {
    pub period_id: Uuid,
    pub as_of: NaiveDate,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConsolidateResponse {
    pub group_id: Uuid,
    pub as_of: String,
    pub reporting_currency: String,
    pub row_count: usize,
    pub rows: Vec<engine::ConsolidatedTbRow>,
    pub input_hash: String,
    pub entity_hashes: Vec<engine::EntityHashEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CachedTbResponse {
    pub group_id: Uuid,
    pub as_of: String,
    pub row_count: usize,
    pub rows: Vec<compute::CachedTbRow>,
    pub source: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CachedTbQuery {
    pub as_of: NaiveDate,
}

#[utoipa::path(
    post, path = "/api/consolidation/groups/{group_id}/consolidate", tag = "Engine",
    params(("group_id" = Uuid, Path)),
    request_body = ConsolidateQuery,
    responses((status = 200, body = ConsolidateResponse), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn run_consolidation(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(params): Json<ConsolidateQuery>,
) -> impl IntoResponse {
    let _tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let gl_client = app_state.gl_client();

    match compute::consolidate(
        &app_state.pool,
        &gl_client,
        &_tenant_id,
        group_id,
        params.period_id,
        params.as_of,
    )
    .await
    {
        Ok(result) => {
            app_state.metrics.consolidation_runs_total.inc();
            Json(ConsolidateResponse {
                group_id: result.group_id,
                as_of: result.as_of.to_string(),
                reporting_currency: result.reporting_currency,
                row_count: result.rows.len(),
                rows: result.rows,
                input_hash: result.input_hash,
                entity_hashes: result.entity_hashes,
            })
            .into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/consolidation/groups/{group_id}/trial-balance", tag = "Engine",
    params(("group_id" = Uuid, Path), CachedTbQuery),
    responses((status = 200, body = CachedTbResponse), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_consolidated_tb(
    State(app_state): State<Arc<AppState>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Query(params): Query<CachedTbQuery>,
) -> impl IntoResponse {
    match compute::get_cached_tb(&app_state.pool, group_id, params.as_of).await {
        Ok(Some(rows)) => Json(CachedTbResponse {
            group_id,
            as_of: params.as_of.to_string(),
            row_count: rows.len(),
            rows,
            source: "cache".to_string(),
        })
        .into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!(
                "No cached TB for group {} as_of {}",
                group_id, params.as_of
            )),
            &ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
