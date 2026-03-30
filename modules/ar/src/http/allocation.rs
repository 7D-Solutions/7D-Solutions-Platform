use axum::{extract::State, Extension, Json};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::ApiError;

// ============================================================================
// Payment Allocation (bd-14f)
// ============================================================================

/// POST /api/ar/payments/allocate — FIFO payment allocation
pub async fn allocate_payment_route(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<crate::payment_allocation::AllocatePaymentRequest>,
) -> Result<Json<crate::payment_allocation::AllocationResult>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let result = crate::payment_allocation::allocate_payment_fifo(&db, &app_id, &req)
        .await
        .map_err(|e| match e {
            crate::payment_allocation::AllocationError::Validation(msg) => {
                ApiError::bad_request(msg)
            }
            crate::payment_allocation::AllocationError::Database(db_err) => {
                tracing::error!("Allocation DB error: {:?}", db_err);
                ApiError::internal("Internal database error")
            }
        })?;

    Ok(Json(result))
}
