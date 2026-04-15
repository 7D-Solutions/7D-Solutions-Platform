use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::fg_receipt::{request_fg_receipt, RequestFgReceiptRequest},
    AppState,
};
use platform_sdk::extract_tenant;

/// POST /api/production/work-orders/:id/fg-receipt
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{id}/fg-receipt",
    tag = "FG Receipts",
    params(("id" = Uuid, Path, description = "Work order ID")),
    request_body = RequestFgReceiptRequest,
    responses(
        (status = 202, description = "FG receipt accepted"),
        (status = 404, description = "Work order not found", body = ApiError),
        (status = 422, description = "Not released or validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_fg_receipt(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<RequestFgReceiptRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match request_fg_receipt(&state.pool, id, &req).await {
        Ok(_replay) => (
            StatusCode::ACCEPTED,
            Json(json!({ "status": "accepted", "work_order_id": id })),
        )
            .into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}
