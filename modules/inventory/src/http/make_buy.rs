//! Make/Buy classification HTTP handler.
//!
//! Endpoints:
//!   PUT /api/inventory/items/:id/make-buy — set make/buy classification

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::make_buy::{set_make_buy, MakeBuyError, SetMakeBuyRequest},
    AppState,
};

fn make_buy_error_response(err: MakeBuyError) -> impl IntoResponse {
    match err {
        MakeBuyError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Item not found" })),
        ),
        MakeBuyError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        MakeBuyError::Serialization(e) => {
            tracing::error!(error = %e, "make_buy serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        MakeBuyError::Database(e) => {
            tracing::error!(error = %e, "make_buy database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// PUT /api/inventory/items/:id/make-buy
///
/// Set or change the make/buy classification on an item.
/// Emits inventory.make_buy_changed event.
pub async fn put_make_buy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<SetMakeBuyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match set_make_buy(&state.pool, id, &req).await {
        Ok(result) => (StatusCode::OK, Json(json!(result.item))).into_response(),
        Err(e) => make_buy_error_response(e).into_response(),
    }
}
