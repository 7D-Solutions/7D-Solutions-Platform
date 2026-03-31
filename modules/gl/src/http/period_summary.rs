//! Period Summary API Routes
//!
//! Provides HTTP endpoints for querying period summary reports.

use crate::AppState;
use axum::{extract::{Path, Query, State}, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::auth::{extract_tenant, with_request_id};
use crate::services::period_summary_service::{self, PeriodSummaryResponse};

#[derive(Debug, Deserialize)]
pub struct PeriodSummaryQuery {
    pub currency: Option<String>,
}

#[utoipa::path(get, path = "/api/gl/periods/{period_id}/summary", tag = "Period Summary",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    responses((status = 200, description = "Period summary report")),
    security(("bearer" = [])))]
pub async fn get_period_summary(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
    Query(params): Query<PeriodSummaryQuery>,
) -> Result<Json<PeriodSummaryResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let response = period_summary_service::get_period_summary(
        &app_state.pool,
        &tenant_id,
        period_id,
        params.currency.as_deref(),
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            period_summary_service::PeriodSummaryServiceError::InvalidTenantId(_)
            | period_summary_service::PeriodSummaryServiceError::InvalidCurrency(_) => {
                ApiError::bad_request(e.to_string())
            }
            period_summary_service::PeriodSummaryServiceError::Repo(ref repo_err) => {
                match repo_err {
                    crate::repos::period_summary_repo::PeriodSummaryError::PeriodNotFound(_) => {
                        ApiError::not_found(e.to_string())
                    }
                    crate::repos::period_summary_repo::PeriodSummaryError::Database(_) => {
                        ApiError::internal(e.to_string())
                    }
                }
            }
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}
