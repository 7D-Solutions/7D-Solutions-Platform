use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use sqlx::PgPool;

use crate::models::ErrorResponse;
use crate::write_offs::{write_off_invoice, WriteOffInvoiceRequest};

// ============================================================================
// WRITE-OFF HANDLER (bd-2f2)
// ============================================================================

/// POST /api/ar/invoices/{id}/write-off
///
/// Write off an invoice as uncollectable bad debt. Idempotent on `write_off_id`.
/// Emits ar.invoice_written_off (REVERSAL) into the outbox atomically.
pub async fn write_off_invoice_route(
    State(db): State<PgPool>,
    Path(invoice_id): Path<i32>,
    Json(mut req): Json<WriteOffInvoiceRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
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
        Ok(WriteOffInvoiceResult::AlreadyWrittenOff { invoice_id }) => Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse::new(
                "already_written_off",
                format!("Invoice {} already has a write-off applied", invoice_id),
            )),
        )),
        Err(e) => {
            tracing::error!("Write-off error: {:?}", e);
            let (status, msg) = match &e {
                crate::write_offs::WriteOffError::DatabaseError(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal database error".to_string(),
                ),
                crate::write_offs::WriteOffError::InvalidAmount(n) => (
                    StatusCode::BAD_REQUEST,
                    format!("Amount must be > 0, got {}", n),
                ),
                other => (StatusCode::BAD_REQUEST, format!("{}", other)),
            };
            Err((status, Json(ErrorResponse::new("write_off_error", msg))))
        }
    }
}
