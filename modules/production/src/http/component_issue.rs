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

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::component_issue::{request_component_issue, RequestComponentIssueRequest},
    AppState,
};

/// POST /api/production/work-orders/:id/component-issues
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{id}/component-issues",
    tag = "Component Issues",
    params(("id" = Uuid, Path, description = "Work order ID")),
    request_body = RequestComponentIssueRequest,
    responses(
        (status = 202, description = "Component issue accepted"),
        (status = 404, description = "Work order not found", body = ApiError),
        (status = 422, description = "Not released or validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_component_issue(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<RequestComponentIssueRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match request_component_issue(&state.pool, id, &req).await {
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
