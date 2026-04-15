//! HTTP handler for bank statement CSV import.
//!
//! POST /api/treasury/statements/import — multipart form upload.
//! Required fields: file (CSV), account_id, period_start, period_end,
//! opening_balance_minor, closing_balance_minor.

use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::import::adapters::CsvFormat;
use crate::domain::import::{service, ImportResult};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

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

async fn collect_fields(mut multipart: Multipart) -> Result<ImportFields, ApiError> {
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
                csv_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("Failed to read file: {}", e)))?
                        .to_vec(),
                );
            }
            "account_id" => {
                let text = field_text(field).await?;
                account_id = Some(
                    text.parse::<Uuid>()
                        .map_err(|_| ApiError::bad_request("account_id must be a valid UUID"))?,
                );
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
                    ApiError::bad_request("opening_balance_minor must be an integer")
                })?);
            }
            "closing_balance_minor" => {
                let text = field_text(field).await?;
                closing_balance = Some(text.trim().parse::<i64>().map_err(|_| {
                    ApiError::bad_request("closing_balance_minor must be an integer")
                })?);
            }
            "format" => {
                let text = field_text(field).await?;
                format = Some(
                    serde_json::from_value::<CsvFormat>(serde_json::Value::String(
                        text.trim().to_string(),
                    ))
                    .map_err(|_| {
                        ApiError::bad_request(
                            "format must be one of: generic, chase_credit, amex_credit",
                        )
                    })?,
                );
            }
            _ => {} // Ignore unknown fields
        }
    }

    let csv_data = csv_data.ok_or_else(|| ApiError::bad_request("file field is required"))?;
    if csv_data.is_empty() {
        return Err(ApiError::bad_request("CSV file is empty"));
    }

    Ok(ImportFields {
        account_id: account_id.ok_or_else(|| ApiError::bad_request("account_id is required"))?,
        period_start: period_start
            .ok_or_else(|| ApiError::bad_request("period_start is required"))?,
        period_end: period_end.ok_or_else(|| ApiError::bad_request("period_end is required"))?,
        opening_balance_minor: opening_balance
            .ok_or_else(|| ApiError::bad_request("opening_balance_minor is required"))?,
        closing_balance_minor: closing_balance
            .ok_or_else(|| ApiError::bad_request("closing_balance_minor is required"))?,
        csv_data,
        filename,
        format,
    })
}

async fn field_text(field: axum::extract::multipart::Field<'_>) -> Result<String, ApiError> {
    field
        .text()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read field: {}", e)))
}

fn parse_date_field(s: &str, name: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| ApiError::bad_request(format!("{} must be YYYY-MM-DD format", name)))
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/treasury/statements/import
#[utoipa::path(
    post, path = "/api/treasury/statements/import", tag = "Import",
    responses(
        (status = 201, description = "Statement imported", body = ImportResult),
        (status = 400, description = "Bad request", body = ApiError),
        (status = 404, description = "Account not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn import_statement(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    let correlation_id = headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let fields = match collect_fields(multipart).await {
        Ok(f) => f,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

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
        Ok(result) => {
            state.metrics.record_import_success();
            (StatusCode::CREATED, Json(result)).into_response()
        }
        Err(e) => {
            state.metrics.record_import_fail();
            with_request_id(ApiError::from(e), &ctx).into_response()
        }
    }
}
