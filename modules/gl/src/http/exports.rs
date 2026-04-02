//! GL Export HTTP handler

use crate::exports::service::{self, ExportFormat, ExportRequest, ExportType};
use crate::AppState;
use axum::{extract::State, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::auth::with_request_id;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateExportRequest {
    pub format: String,
    pub export_type: String,
    pub idempotency_key: String,
    pub period_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExportResponse {
    pub export_id: Uuid,
    pub format: String,
    pub export_type: String,
    pub output: String,
    pub created_at: String,
}

/// POST /api/gl/exports
#[utoipa::path(post, path = "/api/gl/exports", tag = "Exports",
    request_body = CreateExportRequest,
    responses((status = 200, description = "Export created", body = ExportResponse)),
    security(("bearer" = [])))]
pub async fn create_export(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(body): Json<CreateExportRequest>,
) -> Result<Json<ExportResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let format = ExportFormat::from_str(&body.format).ok_or_else(|| {
        with_request_id(
            ApiError::bad_request(format!("Invalid format: {}. Use 'quickbooks' or 'xero'", body.format)),
            &ctx,
        )
    })?;

    let export_type = ExportType::from_str(&body.export_type).ok_or_else(|| {
        with_request_id(
            ApiError::bad_request(format!(
                "Invalid export_type: {}. Use 'chart_of_accounts' or 'journal_entries'",
                body.export_type
            )),
            &ctx,
        )
    })?;

    let req = ExportRequest {
        tenant_id,
        format,
        export_type,
        idempotency_key: body.idempotency_key,
        period_id: body.period_id,
    };

    let result = service::execute_export(&app_state.pool, req)
        .await
        .map_err(|e| {
            let api_err = match &e {
                service::ExportError::InvalidFormat(_)
                | service::ExportError::InvalidExportType(_)
                | service::ExportError::MissingPeriodId => ApiError::bad_request(e.to_string()),
                service::ExportError::DuplicateIdempotencyKey => {
                    ApiError::conflict(e.to_string())
                }
                service::ExportError::Database(_) => ApiError::internal(e.to_string()),
            };
            with_request_id(api_err, &ctx)
        })?;

    Ok(Json(ExportResponse {
        export_id: result.export_id,
        format: result.format,
        export_type: result.export_type,
        output: result.output,
        created_at: result.created_at,
    }))
}
