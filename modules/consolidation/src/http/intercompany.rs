//! HTTP handlers for intercompany matching and elimination posting.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::eliminations::{self, service as elim_service};
use crate::domain::intercompany::{self, service as ic_service};
use crate::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct IntercompanyMatchRequest {
    pub period_id: Uuid,
    pub as_of: NaiveDate,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IntercompanyMatchResponse {
    pub group_id: Uuid,
    pub as_of: String,
    pub match_count: usize,
    pub unmatched_count: usize,
    pub total_matched_minor: i64,
    pub matches: Vec<intercompany::IntercompanyMatch>,
    pub suggestions: Vec<eliminations::EliminationSuggestion>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostEliminationsRequest {
    pub period_id: Uuid,
    pub as_of: NaiveDate,
    pub reporting_currency: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PostEliminationsResponse {
    pub group_id: Uuid,
    pub period_id: Uuid,
    pub posted_count: usize,
    pub idempotency_key: String,
    pub journal_entry_ids: Vec<Uuid>,
    pub already_posted: bool,
}

#[utoipa::path(
    post, path = "/api/consolidation/groups/{group_id}/intercompany-match", tag = "Intercompany",
    params(("group_id" = Uuid, Path)),
    request_body = IntercompanyMatchRequest,
    responses((status = 200, body = IntercompanyMatchResponse), (status = 500, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn run_intercompany_match(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(params): Json<IntercompanyMatchRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let gl_client = app_state.gl_client();

    match ic_service::match_intercompany_for_group(
        &app_state.pool,
        &gl_client,
        &tenant_id,
        group_id,
        params.period_id,
        params.as_of,
    )
    .await
    {
        Ok(match_result) => {
            let suggestions = elim_service::suggest_eliminations(&match_result);
            Json(IntercompanyMatchResponse {
                group_id,
                as_of: params.as_of.to_string(),
                match_count: match_result.matches.len(),
                unmatched_count: match_result.unmatched_count,
                total_matched_minor: match_result.total_matched_minor,
                matches: match_result.matches,
                suggestions,
            })
            .into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/consolidation/groups/{group_id}/eliminations", tag = "Intercompany",
    params(("group_id" = Uuid, Path)),
    request_body = PostEliminationsRequest,
    responses((status = 200, body = PostEliminationsResponse), (status = 500, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn post_eliminations(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(group_id): Path<Uuid>,
    Json(params): Json<PostEliminationsRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let gl_client = app_state.gl_client();

    let match_result = match ic_service::match_intercompany_for_group(
        &app_state.pool,
        &gl_client,
        &tenant_id,
        group_id,
        params.period_id,
        params.as_of,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return with_request_id(ApiError::from(e), &ctx).into_response(),
    };

    let suggestions = elim_service::suggest_eliminations(&match_result);

    match elim_service::post_eliminations(
        &app_state.pool,
        &gl_client,
        &tenant_id,
        group_id,
        params.period_id,
        params.as_of,
        &suggestions,
        &params.reporting_currency,
    )
    .await
    {
        Ok(post_result) => Json(PostEliminationsResponse {
            group_id,
            period_id: params.period_id,
            posted_count: post_result.posted_count,
            idempotency_key: post_result.idempotency_key,
            journal_entry_ids: post_result.journal_entry_ids,
            already_posted: post_result.already_posted,
        })
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
