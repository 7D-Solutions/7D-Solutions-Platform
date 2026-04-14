use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use sqlx::PgPool;

use crate::models::ApiError;
use crate::write_offs::{write_off_invoice, WriteOffInvoiceRequest};

// ============================================================================
// WRITE-OFF HANDLER (bd-2f2)
// ============================================================================

#[utoipa::path(post, path = "/api/ar/invoices/{id}/write-off", tag = "Write-offs",
    params(("id" = i32, Path, description = "Invoice ID")),
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Invoice written off", body = serde_json::Value),
        (status = 200, description = "Already processed (idempotent)", body = serde_json::Value),
        (status = 409, description = "Already written off", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
/// POST /api/ar/invoices/{id}/write-off
///
/// Write off an invoice as uncollectable bad debt. Idempotent on `write_off_id`.
/// Emits ar.invoice_written_off (REVERSAL) into the outbox atomically.
pub async fn write_off_invoice_route(
    State(db): State<PgPool>,
    Path(invoice_id): Path<i32>,
    Json(mut req): Json<WriteOffInvoiceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    use crate::write_offs::WriteOffInvoiceResult;
    req.invoice_id = invoice_id;
    match write_off_invoice(&db, req).await {
        Ok(WriteOffInvoiceResult::WrittenOff {
            write_off_row_id,
            write_off_id,
            written_off_at,
        }) => Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "written_off",
                "write_off_row_id": write_off_row_id,
                "write_off_id": write_off_id,
                "written_off_at": written_off_at,
            })),
        )),
        Ok(WriteOffInvoiceResult::AlreadyProcessed {
            existing_row_id,
            write_off_id,
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "already_processed",
                "existing_row_id": existing_row_id,
                "write_off_id": write_off_id,
            })),
        )),
        Ok(WriteOffInvoiceResult::AlreadyWrittenOff { invoice_id }) => Err(ApiError::conflict(
            format!("Invoice {} already has a write-off applied", invoice_id),
        )),
        Err(e) => {
            tracing::error!(error = %e, "Write-off error");
            match &e {
                crate::write_offs::WriteOffError::DatabaseError(_) => {
                    Err(ApiError::internal("Internal database error"))
                }
                crate::write_offs::WriteOffError::InvalidAmount(n) => Err(ApiError::bad_request(
                    format!("Amount must be > 0, got {}", n),
                )),
                other => Err(ApiError::bad_request(format!("{}", other))),
            }
        }
    }
}
