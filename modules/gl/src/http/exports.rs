//! GL Export HTTP handler

use crate::exports::service::{self, ExportFormat, ExportRequest, ExportType};
use crate::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::auth::extract_tenant;

#[derive(Debug, Deserialize)]
pub struct CreateExportRequest {
    pub format: String,
    pub export_type: String,
    pub idempotency_key: String,
    pub period_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ExportResponse {
    pub export_id: Uuid,
    pub format: String,
    pub export_type: String,
    pub output: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ExportErrorBody {
    pub error: String,
}

pub struct ExportErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for ExportErrorResponse {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ExportErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

/// POST /api/gl/exports
pub async fn create_export(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<CreateExportRequest>,
) -> Result<Json<ExportResponse>, ExportErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| ExportErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let format = ExportFormat::from_str(&body.format).ok_or_else(|| ExportErrorResponse {
        status: StatusCode::BAD_REQUEST,
        message: format!("Invalid format: {}. Use 'quickbooks' or 'xero'", body.format),
    })?;

    let export_type =
        ExportType::from_str(&body.export_type).ok_or_else(|| ExportErrorResponse {
            status: StatusCode::BAD_REQUEST,
            message: format!(
                "Invalid export_type: {}. Use 'chart_of_accounts' or 'journal_entries'",
                body.export_type
            ),
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
            let status = match &e {
                service::ExportError::InvalidFormat(_) | service::ExportError::InvalidExportType(_) => {
                    StatusCode::BAD_REQUEST
                }
                service::ExportError::MissingPeriodId => StatusCode::BAD_REQUEST,
                service::ExportError::DuplicateIdempotencyKey => StatusCode::CONFLICT,
                service::ExportError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            ExportErrorResponse {
                status,
                message: e.to_string(),
            }
        })?;

    Ok(Json(ExportResponse {
        export_id: result.export_id,
        format: result.format,
        export_type: result.export_type,
        output: result.output,
        created_at: result.created_at,
    }))
}
