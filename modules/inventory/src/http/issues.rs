//! Stock issue HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/issues — atomic issue: ledger row + FIFO layer consumptions
//!                                + on-hand projection + outbox event (inventory.item_issued)
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
    domain::issue_service::{process_issue, IssueRequest, IssueResult},
    AppState,
};

// ============================================================================
// Handler
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/issues",
    tag = "Issues",
    request_body = IssueRequest,
    responses(
        (status = 201, description = "Issue created", body = IssueResult),
        (status = 200, description = "Idempotency replay", body = IssueResult),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
        (status = 404, description = "Item not found or wrong tenant", body = ApiError),
        (status = 422, description = "Validation failure or insufficient stock", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_issue(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<IssueRequest>,
) -> impl IntoResponse {
    let tracing_ctx_val = tracing_ctx.as_ref().map(|Extension(c)| c.clone());
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match process_issue(&state.pool, &req, tracing_ctx_val.as_ref()).await {
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
    fn placeholder_issues_module_compiles() {}
}
