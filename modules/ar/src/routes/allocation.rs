use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use sqlx::PgPool;

use crate::models::ErrorResponse;

// ============================================================================
// Payment Allocation (bd-14f)
// ============================================================================

/// POST /api/ar/payments/allocate — FIFO payment allocation
pub async fn allocate_payment_route(
    State(db): State<PgPool>,
    Json(req): Json<crate::payment_allocation::AllocatePaymentRequest>,
) -> Result<Json<crate::payment_allocation::AllocationResult>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default-tenant";

    let result = crate::payment_allocation::allocate_payment_fifo(&db, app_id, &req)
        .await
        .map_err(|e| match e {
            crate::payment_allocation::AllocationError::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("validation_error", msg)),
            ),
            crate::payment_allocation::AllocationError::Database(db_err) => {
                tracing::error!("Allocation DB error: {:?}", db_err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new("database_error", format!("{}", db_err))),
                )
            }
        })?;

    Ok(Json(result))
}
