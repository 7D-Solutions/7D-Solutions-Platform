use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::services::fx_rate_service;
use crate::AppState;
use super::auth::extract_tenant;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateFxRateRequest {
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub effective_at: DateTime<Utc>,
    pub source: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct CreateFxRateResponse {
    pub rate_id: Uuid,
    pub created: bool,
}

#[derive(Debug, Deserialize)]
pub struct LatestRateQuery {
    pub base_currency: String,
    pub quote_currency: String,
    pub as_of: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct FxRateResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub inverse_rate: f64,
    pub effective_at: DateTime<Utc>,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/gl/fx-rates
///
/// Create a new FX rate. Duplicate idempotency_key returns 200 with created=false.
pub async fn create_fx_rate(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateFxRateRequest>,
) -> Result<Json<CreateFxRateResponse>, FxRateErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| FxRateErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let svc_req = fx_rate_service::CreateFxRateRequest {
        tenant_id,
        base_currency: req.base_currency.to_uppercase(),
        quote_currency: req.quote_currency.to_uppercase(),
        rate: req.rate,
        effective_at: req.effective_at,
        source: req.source,
        idempotency_key: req.idempotency_key,
    };

    let result = fx_rate_service::create_fx_rate(&app_state.pool, svc_req)
        .await
        .map_err(|e| FxRateErrorResponse {
            status: StatusCode::BAD_REQUEST,
            message: e,
        })?;

    Ok(Json(CreateFxRateResponse {
        rate_id: result.rate_id,
        created: result.was_inserted,
    }))
}

/// GET /api/gl/fx-rates/latest
///
/// Returns the latest rate for a currency pair as-of a given time (default: now).
pub async fn get_latest_rate(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<LatestRateQuery>,
) -> Result<Json<FxRateResponse>, FxRateErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| FxRateErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let as_of = params.as_of.unwrap_or_else(Utc::now);

    let rate = fx_rate_service::get_latest_rate(
        &app_state.pool,
        &tenant_id,
        &params.base_currency.to_uppercase(),
        &params.quote_currency.to_uppercase(),
        as_of,
    )
    .await
    .map_err(|e| FxRateErrorResponse {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: e,
    })?
    .ok_or_else(|| FxRateErrorResponse {
        status: StatusCode::NOT_FOUND,
        message: format!(
            "No FX rate found for {}/{} as of {}",
            params.base_currency, params.quote_currency, as_of
        ),
    })?;

    Ok(Json(FxRateResponse {
        id: rate.id,
        tenant_id: rate.tenant_id,
        base_currency: rate.base_currency,
        quote_currency: rate.quote_currency,
        rate: rate.rate,
        inverse_rate: rate.inverse_rate,
        effective_at: rate.effective_at,
        source: rate.source,
        created_at: rate.created_at,
    }))
}

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug)]
pub struct FxRateErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for FxRateErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}
