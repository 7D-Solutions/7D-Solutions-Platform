//! Stock receipt HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/receipts — atomic receipt: ledger row + FIFO layer + outbox event
//!
//! Tenant identity derived from JWT `VerifiedClaims`.
//!
//! Idempotency:
//!   Callers MUST supply `idempotency_key` in the request body.
//!   Duplicate keys with the same body return 200 OK with the stored result.
//!   Duplicate keys with a different body return 409 Conflict.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::receipt_service::{process_receipt, ReceiptRequest},
    AppState,
};

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/receipts
///
/// Creates a stock receipt atomically: ledger row + FIFO layer + outbox event,
/// all in a single database transaction.
///
/// Responses:
///   201 Created  — receipt created, rows committed
///   200 OK       — idempotency replay (same key + same body, stored result returned)
///   409 Conflict — same idempotency key with a different request body
///   404 Not Found — item not found or wrong tenant
///   422 Unprocessable Entity — validation failure (inactive item, zero qty, zero cost)
///   500 Internal Server Error — unexpected error
pub async fn post_receipt(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<ReceiptRequest>,
) -> impl IntoResponse {
    let tracing_ctx_val = tracing_ctx.as_ref().map(|Extension(c)| c.clone());
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match process_receipt(&state.pool, &req, tracing_ctx_val.as_ref()).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(result)).into_response()
        }
        Err(err) => {
            let api_err: ApiError = err.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_receipts_module_compiles() {}
}
