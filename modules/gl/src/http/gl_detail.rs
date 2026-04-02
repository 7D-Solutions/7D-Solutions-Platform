//! GL Detail API Routes
//!
//! Provides HTTP endpoints for querying GL detail reports (journal entries and lines).

use crate::AppState;
use axum::{extract::{Query, State}, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::auth::with_request_id;
use crate::services::gl_detail_service::{self, GLDetailResponse};

/// Query parameters for GL detail endpoint
#[derive(Debug, Deserialize)]
pub struct GLDetailQuery {
    pub period_id: Uuid,
    pub account_code: Option<String>,
    pub currency: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[utoipa::path(get, path = "/api/gl/detail", tag = "GL Detail",
    responses((status = 200, description = "GL detail report", body = GLDetailResponse)),
    security(("bearer" = [])))]
pub async fn get_gl_detail(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<GLDetailQuery>,
) -> Result<Json<GLDetailResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let response = gl_detail_service::get_gl_detail(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        params.account_code.as_deref(),
        params.currency.as_deref(),
        params.limit,
        params.offset,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            gl_detail_service::GLDetailServiceError::PeriodNotFound { .. } => {
                ApiError::not_found(e.to_string())
            }
            gl_detail_service::GLDetailServiceError::Repo(_) => {
                ApiError::internal(e.to_string())
            }
            _ => ApiError::bad_request(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}
