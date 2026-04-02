//! HTTP handler for AP paid-tax reporting (bd-1ai1).
//!
//! GET /api/ap/tax/reports/summary — Paid-tax summaries by period/jurisdiction
//! GET /api/ap/tax/reports/export  — Deterministic CSV or JSON export

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use std::sync::Arc;

use crate::domain::tax::reports;
use platform_sdk::extract_tenant;
use crate::http::tenant::with_request_id;
use crate::AppState;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TaxReportQuery {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TaxExportQuery {
    pub from: NaiveDate,
    pub to: NaiveDate,
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "json".to_string()
}

// ============================================================================
// Response types
// ============================================================================

#[derive(Debug, Serialize, ToSchema)]
pub struct ApTaxReportResponse {
    pub tenant_id: String,
    pub from: String,
    pub to: String,
    pub rows: Vec<reports::ApTaxSummaryRow>,
    pub total_tax_minor: i64,
    pub total_bills: i64,
}

// ============================================================================
// GET /api/ap/tax/reports/summary
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/ap/tax/reports/summary",
    tag = "Tax Reports",
    params(TaxReportQuery),
    responses(
        (status = 200, description = "Paid-tax summary by period/jurisdiction", body = ApTaxReportResponse),
        (status = 400, description = "Invalid date range", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn tax_report_summary(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<TaxReportQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    if params.from >= params.to {
        return with_request_id(
            ApiError::bad_request("`from` must be before `to`"),
            &tracing_ctx,
        )
        .into_response();
    }

    let rows =
        match reports::ap_tax_summary_by_period(&state.pool, &tenant_id, params.from, params.to)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "ap tax_report_summary DB error");
                return with_request_id(
                    ApiError::internal("An internal error occurred"),
                    &tracing_ctx,
                )
                .into_response();
            }
        };

    let total_tax: i64 = rows.iter().map(|r| r.total_tax_minor).sum();
    let total_bills: i64 = rows.iter().map(|r| r.bill_count).sum();

    Json(ApTaxReportResponse {
        tenant_id,
        from: params.from.to_string(),
        to: params.to.to_string(),
        rows,
        total_tax_minor: total_tax,
        total_bills,
    })
    .into_response()
}

// ============================================================================
// GET /api/ap/tax/reports/export
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/ap/tax/reports/export",
    tag = "Tax Reports",
    params(TaxExportQuery),
    responses(
        (status = 200, description = "Tax report export (JSON or CSV)", body = ApTaxReportResponse),
        (status = 400, description = "Invalid date range", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn tax_report_export(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<TaxExportQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    if params.from >= params.to {
        return with_request_id(
            ApiError::bad_request("`from` must be before `to`"),
            &tracing_ctx,
        )
        .into_response();
    }

    let rows =
        match reports::ap_tax_summary_by_period(&state.pool, &tenant_id, params.from, params.to)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "ap tax_report_export DB error");
                return with_request_id(
                    ApiError::internal("An internal error occurred"),
                    &tracing_ctx,
                )
                .into_response();
            }
        };

    match params.format.as_str() {
        "csv" => {
            let csv = reports::render_csv(&rows);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                csv,
            )
                .into_response()
        }
        _ => {
            let total_tax: i64 = rows.iter().map(|r| r.total_tax_minor).sum();
            let total_bills: i64 = rows.iter().map(|r| r.bill_count).sum();

            Json(ApTaxReportResponse {
                tenant_id,
                from: params.from.to_string(),
                to: params.to.to_string(),
                rows,
                total_tax_minor: total_tax,
                total_bills,
            })
            .into_response()
        }
    }
}
