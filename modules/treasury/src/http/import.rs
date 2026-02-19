//! HTTP handler for bank statement CSV import.
//!
//! POST /api/treasury/statements/import — multipart form upload.
//! Required fields: file (CSV), account_id, period_start, period_end,
//! opening_balance_minor, closing_balance_minor.

use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::NaiveDate;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::import::adapters::CsvFormat;
use crate::domain::import::{service, ImportError, ImportResult, LineError};
use crate::AppState;

// ============================================================================
// Error response
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ImportErrorBody {
    error: String,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    line_errors: Vec<LineError>,
}

impl ImportErrorBody {
    fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
            line_errors: vec![],
        }
    }

    fn with_lines(error: &str, message: &str, errors: Vec<LineError>) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
            line_errors: errors,
        }
    }
}

// ============================================================================
// Multipart field collector
// ============================================================================

struct ImportFields {
    account_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    opening_balance_minor: i64,
    closing_balance_minor: i64,
    csv_data: Vec<u8>,
    filename: Option<String>,
    format: Option<CsvFormat>,
}

async fn collect_fields(
    mut multipart: Multipart,
) -> Result<ImportFields, (StatusCode, Json<ImportErrorBody>)> {
    let mut account_id: Option<Uuid> = None;
    let mut period_start: Option<NaiveDate> = None;
    let mut period_end: Option<NaiveDate> = None;
    let mut opening_balance: Option<i64> = None;
    let mut closing_balance: Option<i64> = None;
    let mut csv_data: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut format: Option<CsvFormat> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                filename = field.file_name().map(String::from);
                csv_data = Some(field.bytes().await.map_err(|e| {
                    bad_request(&format!("Failed to read file: {}", e))
                })?.to_vec());
            }
            "account_id" => {
                let text = field_text(field).await?;
                account_id = Some(text.parse::<Uuid>().map_err(|_| {
                    bad_request("account_id must be a valid UUID")
                })?);
            }
            "period_start" => {
                let text = field_text(field).await?;
                period_start = Some(parse_date_field(&text, "period_start")?);
            }
            "period_end" => {
                let text = field_text(field).await?;
                period_end = Some(parse_date_field(&text, "period_end")?);
            }
            "opening_balance_minor" => {
                let text = field_text(field).await?;
                opening_balance = Some(text.trim().parse::<i64>().map_err(|_| {
                    bad_request("opening_balance_minor must be an integer")
                })?);
            }
            "closing_balance_minor" => {
                let text = field_text(field).await?;
                closing_balance = Some(text.trim().parse::<i64>().map_err(|_| {
                    bad_request("closing_balance_minor must be an integer")
                })?);
            }
            "format" => {
                let text = field_text(field).await?;
                format = Some(serde_json::from_value::<CsvFormat>(
                    serde_json::Value::String(text.trim().to_string()),
                ).map_err(|_| {
                    bad_request(
                        "format must be one of: generic, chase_credit, amex_credit",
                    )
                })?);
            }
            _ => {} // Ignore unknown fields
        }
    }

    let csv_data = csv_data.ok_or_else(|| bad_request("file field is required"))?;
    if csv_data.is_empty() {
        return Err(bad_request("CSV file is empty"));
    }

    Ok(ImportFields {
        account_id: account_id.ok_or_else(|| bad_request("account_id is required"))?,
        period_start: period_start.ok_or_else(|| bad_request("period_start is required"))?,
        period_end: period_end.ok_or_else(|| bad_request("period_end is required"))?,
        opening_balance_minor: opening_balance
            .ok_or_else(|| bad_request("opening_balance_minor is required"))?,
        closing_balance_minor: closing_balance
            .ok_or_else(|| bad_request("closing_balance_minor is required"))?,
        csv_data,
        filename,
        format,
    })
}

async fn field_text(
    field: axum::extract::multipart::Field<'_>,
) -> Result<String, (StatusCode, Json<ImportErrorBody>)> {
    field
        .text()
        .await
        .map_err(|e| bad_request(&format!("Failed to read field: {}", e)))
}

fn parse_date_field(
    s: &str,
    name: &str,
) -> Result<NaiveDate, (StatusCode, Json<ImportErrorBody>)> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| bad_request(&format!("{} must be YYYY-MM-DD format", name)))
}

fn bad_request(msg: &str) -> (StatusCode, Json<ImportErrorBody>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ImportErrorBody::new("bad_request", msg)),
    )
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/treasury/statements/import
pub async fn import_statement(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<(StatusCode, Json<ImportResult>), (StatusCode, Json<ImportErrorBody>)> {
    let app_id = headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad_request("X-App-Id header is required"))?
        .to_string();

    let correlation_id = headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let fields = collect_fields(multipart).await?;

    let req = service::ImportRequest {
        account_id: fields.account_id,
        period_start: fields.period_start,
        period_end: fields.period_end,
        opening_balance_minor: fields.opening_balance_minor,
        closing_balance_minor: fields.closing_balance_minor,
        csv_data: fields.csv_data,
        filename: fields.filename,
        format: fields.format,
    };

    match service::import_statement(&state.pool, &app_id, req, correlation_id).await {
        Ok(result) => Ok((StatusCode::CREATED, Json(result))),
        Err(e) => Err(import_error_response(e)),
    }
}

fn import_error_response(e: ImportError) -> (StatusCode, Json<ImportErrorBody>) {
    match e {
        ImportError::AccountNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ImportErrorBody::new(
                "account_not_found",
                &format!("Bank account {} not found", id),
            )),
        ),
        ImportError::AccountNotActive => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ImportErrorBody::new(
                "account_not_active",
                "Bank account is not active",
            )),
        ),
        ImportError::DuplicateImport { statement_id } => (
            StatusCode::OK,
            Json(ImportErrorBody::new(
                "duplicate_import",
                &format!(
                    "Statement already imported with id {}. No duplicates created.",
                    statement_id
                ),
            )),
        ),
        ImportError::EmptyImport => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ImportErrorBody::new(
                "empty_import",
                "CSV contains no transaction lines",
            )),
        ),
        ImportError::AllLinesFailed(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ImportErrorBody::with_lines(
                "all_lines_failed",
                "Every CSV line failed validation",
                errors,
            )),
        ),
        ImportError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ImportErrorBody::new("validation_error", &msg)),
        ),
        ImportError::Database(e) => {
            tracing::error!("Statement import DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ImportErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}
