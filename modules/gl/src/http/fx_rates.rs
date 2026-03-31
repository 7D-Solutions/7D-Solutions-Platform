use axum::{extract::{Query, State}, Extension, Json};
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::auth::{extract_tenant, with_request_id};
use crate::services::fx_rate_service;
use crate::AppState;

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

/// POST /api/gl/fx-rates
#[utoipa::path(post, path = "/api/gl/fx-rates", tag = "FX Rates",
    responses((status = 200, description = "FX rate created")),
    security(("bearer" = [])))]
pub async fn create_fx_rate(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateFxRateRequest>,
) -> Result<Json<CreateFxRateResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

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
        .map_err(|e| with_request_id(ApiError::bad_request(e), &ctx))?;

    Ok(Json(CreateFxRateResponse {
        rate_id: result.rate_id,
        created: result.was_inserted,
    }))
}

/// GET /api/gl/fx-rates/latest
#[utoipa::path(get, path = "/api/gl/fx-rates/latest", tag = "FX Rates",
    responses((status = 200, description = "Latest FX rate")),
    security(("bearer" = [])))]
pub async fn get_latest_rate(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<LatestRateQuery>,
) -> Result<Json<FxRateResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let as_of = params.as_of.unwrap_or_else(Utc::now);

    let rate = fx_rate_service::get_latest_rate(
        &app_state.pool,
        &tenant_id,
        &params.base_currency.to_uppercase(),
        &params.quote_currency.to_uppercase(),
        as_of,
    )
    .await
    .map_err(|e| with_request_id(ApiError::internal(e), &ctx))?
    .ok_or_else(|| {
        with_request_id(
            ApiError::not_found(format!(
                "No FX rate found for {}/{} as of {}",
                params.base_currency, params.quote_currency, as_of
            )),
            &ctx,
        )
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
