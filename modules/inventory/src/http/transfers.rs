//! Stock transfer HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/transfers — move stock between warehouses (paired ledger entries)
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
    domain::transfer_service::{process_transfer, TransferRequest},
    AppState,
};

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/transfers
///
/// Moves stock between warehouses as paired ledger entries (transfer_out + transfer_in)
/// in a single atomic transaction. FIFO is consumed on the source side; a new cost
/// layer is created at the destination at weighted average cost.
///
/// Returns 201 Created on new transfer; 200 OK on idempotency replay.
pub async fn post_transfer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<TransferRequest>,
) -> impl IntoResponse {
    let tracing_ctx_val = tracing_ctx.as_ref().map(|Extension(c)| c.clone());
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match process_transfer(&state.pool, &req, tracing_ctx_val.as_ref()).await {
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
