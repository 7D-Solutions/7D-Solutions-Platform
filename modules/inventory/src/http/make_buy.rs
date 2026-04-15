//! Make/Buy classification HTTP handler.

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
    domain::items::Item,
    domain::make_buy::{set_make_buy, SetMakeBuyRequest},
    AppState,
};
use platform_sdk::extract_tenant;

#[utoipa::path(
    put,
    path = "/api/inventory/items/{id}/make-buy",
    tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    request_body = SetMakeBuyRequest,
    responses(
        (status = 200, description = "Make/buy classification updated", body = Item),
        (status = 404, description = "Item not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn put_make_buy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<SetMakeBuyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match set_make_buy(&state.pool, id, &req).await {
        Ok(result) => (StatusCode::OK, Json(json!(result.item))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}
