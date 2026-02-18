//! HTTP handlers for treasury reports.

use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};
use std::sync::Arc;

use super::accounts::ErrorBody;
use crate::domain::reports::cash_position;
use crate::AppState;

fn app_id_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_app_id", "X-App-Id header is required")),
            )
        })
}

/// GET /api/treasury/cash-position — real-time cash position by account and currency
pub async fn cash_position(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<cash_position::CashPositionResponse>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let position = cash_position::get_cash_position(&state.pool, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Cash position query failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Failed to compute cash position")),
            )
        })?;

    Ok(Json(position))
}
