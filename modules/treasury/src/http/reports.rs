//! HTTP handlers for treasury reports.
//!
//! Tenant identity is derived from JWT claims via [`VerifiedClaims`].

use axum::{extract::State, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;

use crate::domain::reports::cash_position;
use crate::domain::reports::{assumptions::ForecastAssumptions, forecast};
use crate::http::tenant::{extract_tenant, with_request_id};
use crate::AppState;

/// GET /api/treasury/cash-position — real-time cash position by account and currency
#[utoipa::path(
    get, path = "/api/treasury/cash-position", tag = "Reports",
    responses(
        (status = 200, description = "Cash position report", body = cash_position::CashPositionResponse),
    ),
    security(("bearer" = [])),
)]
pub async fn cash_position(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    match cash_position::get_cash_position(&state.pool, &app_id).await {
        Ok(position) => Json(position).into_response(),
        Err(e) => {
            tracing::error!("Cash position query failed: {}", e);
            with_request_id(ApiError::internal("Failed to compute cash position"), &ctx)
                .into_response()
        }
    }
}

/// GET /api/treasury/forecast — cash forecast from AR/AP aging + scheduled payments
#[utoipa::path(
    get, path = "/api/treasury/forecast", tag = "Reports",
    responses(
        (status = 200, description = "Cash forecast", body = forecast::ForecastResponse),
    ),
    security(("bearer" = [])),
)]
pub async fn forecast(
    State(_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
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

    Json(response).into_response()
}
