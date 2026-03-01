//! Tax reporting HTTP routes (bd-1ai1)
//!
//! GET /api/ar/tax/reports/summary — Tax collected summaries by period/jurisdiction
//! GET /api/ar/tax/reports/export  — Deterministic CSV or JSON export

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::tax::reporting;

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
    /// "csv" or "json" (default: "json")
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
pub struct TaxReportResponse {
    pub app_id: String,
    pub from: String,
    pub to: String,
    pub rows: Vec<reporting::TaxSummaryRow>,
    pub total_tax_minor: i64,
    pub total_invoices: i64,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

// ============================================================================
// GET /api/ar/tax/reports/summary
// ============================================================================

/// Returns AR collected-tax summaries grouped by period and jurisdiction.
pub async fn tax_report_summary(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<TaxReportQuery>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if params.from >= params.to {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "`from` must be before `to`".to_string(),
            }),
        )
            .into_response();
    }

    match reporting::tax_summary_by_period(&pool, &app_id, params.from, params.to).await {
        Ok(rows) => {
            let total_tax: i64 = rows.iter().map(|r| r.total_tax_minor).sum();
            let total_inv: i64 = rows.iter().map(|r| r.invoice_count).sum();

            let resp = TaxReportResponse {
                app_id,
                from: params.from.to_string(),
                to: params.to.to_string(),
                rows,
                total_tax_minor: total_tax,
                total_invoices: total_inv,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "tax_report_summary DB error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Internal error".to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// GET /api/ar/tax/reports/export
// ============================================================================

/// Returns AR collected-tax summaries as a deterministic CSV or JSON file.
pub async fn tax_report_export(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<TaxExportQuery>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if params.from >= params.to {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "`from` must be before `to`".to_string(),
            }),
        )
            .into_response();
    }

    let rows = match reporting::tax_summary_by_period(&pool, &app_id, params.from, params.to).await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "tax_report_export DB error");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Internal error".to_string(),
                }),
            )
                .into_response();
        }
    };

    match params.format.as_str() {
        "csv" => {
            let csv = reporting::render_csv(&rows);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                csv,
            )
                .into_response()
        }
        _ => {
            let total_tax: i64 = rows.iter().map(|r| r.total_tax_minor).sum();
            let total_inv: i64 = rows.iter().map(|r| r.invoice_count).sum();

            let resp = TaxReportResponse {
                app_id,
                from: params.from.to_string(),
                to: params.to.to_string(),
                rows,
                total_tax_minor: total_tax,
                total_invoices: total_inv,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
    }
}
