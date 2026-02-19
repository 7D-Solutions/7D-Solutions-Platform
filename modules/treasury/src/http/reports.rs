//! HTTP handlers for treasury reports.

use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};
use std::sync::Arc;

use super::accounts::ErrorBody;
use crate::domain::reports::cash_position;
use crate::domain::reports::{assumptions::ForecastAssumptions, forecast};
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

/// GET /api/treasury/forecast — cash forecast from AR/AP aging + scheduled payments
///
/// Reads AR aging (expected inflows) and AP aging (expected outflows) from
/// their respective databases via `AR_DATABASE_URL` and `AP_DATABASE_URL`.
/// If either env var is unset, that data source is skipped and the forecast
/// only includes the available data.
pub async fn forecast(
    State(_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<forecast::ForecastResponse>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let assumptions = ForecastAssumptions::default();
    let mut data_sources = Vec::new();

    // Read AR aging (optional — requires AR_DATABASE_URL)
    let ar_aging = match std::env::var("AR_DATABASE_URL") {
        Ok(url) => match sqlx::PgPool::connect(&url).await {
            Ok(pool) => match forecast::read_ar_aging(&pool, &app_id).await {
                Ok(data) => {
                    data_sources.push("ar_aging_buckets".to_string());
                    data
                }
                Err(e) => {
                    tracing::warn!("Failed to read AR aging: {}", e);
                    vec![]
                }
            },
            Err(e) => {
                tracing::warn!("Failed to connect to AR database: {}", e);
                vec![]
            }
        },
        Err(_) => {
            tracing::info!("AR_DATABASE_URL not set, skipping AR aging in forecast");
            vec![]
        }
    };

    // Read AP aging + scheduled payments (optional — requires AP_DATABASE_URL)
    let (ap_aging, scheduled) = match std::env::var("AP_DATABASE_URL") {
        Ok(url) => match sqlx::PgPool::connect(&url).await {
            Ok(pool) => {
                let aging = match forecast::read_ap_aging(&pool, &app_id).await {
                    Ok(data) => {
                        data_sources.push("ap_vendor_bills".to_string());
                        data
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read AP aging: {}", e);
                        vec![]
                    }
                };
                let sched = match forecast::read_scheduled_payments(&pool, &app_id).await {
                    Ok(data) => {
                        if !data.is_empty() {
                            data_sources.push("ap_payment_runs".to_string());
                        }
                        data
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read AP scheduled payments: {}", e);
                        vec![]
                    }
                };
                (aging, sched)
            }
            Err(e) => {
                tracing::warn!("Failed to connect to AP database: {}", e);
                (vec![], vec![])
            }
        },
        Err(_) => {
            tracing::info!("AP_DATABASE_URL not set, skipping AP aging in forecast");
            (vec![], vec![])
        }
    };

    let response =
        forecast::compute_forecast(&ar_aging, &ap_aging, &scheduled, &assumptions, data_sources);

    Ok(Json(response))
}
