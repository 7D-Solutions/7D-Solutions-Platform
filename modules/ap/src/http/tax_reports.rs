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
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::tax::reports;
use crate::http::admin_types::ErrorBody;
use crate::http::tenant::extract_tenant;
use crate::AppState;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct TaxReportQuery {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Serialize)]
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

pub async fn tax_report_summary(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<TaxReportQuery>,
) -> Result<Json<ApTaxReportResponse>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    if params.from >= params.to {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(
                "invalid_range",
                "`from` must be before `to`",
            )),
        ));
    }

    let rows = reports::ap_tax_summary_by_period(&state.pool, &tenant_id, params.from, params.to)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "ap tax_report_summary DB error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new(
                    "database_error",
                    "An internal error occurred",
                )),
            )
        })?;

    let total_tax: i64 = rows.iter().map(|r| r.total_tax_minor).sum();
    let total_bills: i64 = rows.iter().map(|r| r.bill_count).sum();

    Ok(Json(ApTaxReportResponse {
        tenant_id,
        from: params.from.to_string(),
        to: params.to.to_string(),
        rows,
        total_tax_minor: total_tax,
        total_bills,
    }))
}

// ============================================================================
// GET /api/ap/tax/reports/export
// ============================================================================

pub async fn tax_report_export(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<TaxExportQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    if params.from >= params.to {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(
                "invalid_range",
                "`from` must be before `to`",
            )),
        ));
    }

    let rows = reports::ap_tax_summary_by_period(&state.pool, &tenant_id, params.from, params.to)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "ap tax_report_export DB error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new(
                    "database_error",
                    "An internal error occurred",
                )),
            )
        })?;

    match params.format.as_str() {
        "csv" => {
            let csv = reports::render_csv(&rows);
            Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                csv,
            )
                .into_response())
        }
        _ => {
            let total_tax: i64 = rows.iter().map(|r| r.total_tax_minor).sum();
            let total_bills: i64 = rows.iter().map(|r| r.bill_count).sum();

            Ok(Json(ApTaxReportResponse {
                tenant_id,
                from: params.from.to_string(),
                to: params.to.to_string(),
                rows,
                total_tax_minor: total_tax,
                total_bills,
            })
            .into_response())
        }
    }
}
