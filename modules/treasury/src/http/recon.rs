//! HTTP handlers for bank reconciliation.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::recon::{
    models::{AutoMatchRequest, AutoMatchResult, ListMatchesQuery, ManualMatchRequest, ReconMatch},
    service,
};
use crate::http::tenant::{extract_tenant, with_request_id};
use crate::AppState;

// ============================================================================
// Helpers
// ============================================================================

fn correlation(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/treasury/recon/auto-match — run auto-match for an account
#[utoipa::path(
    post, path = "/api/treasury/recon/auto-match", tag = "Reconciliation",
    request_body = AutoMatchRequest,
    responses(
        (status = 200, description = "Auto-match result", body = AutoMatchResult),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn auto_match(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<AutoMatchRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let correlation_id = correlation(&headers);

    match service::run_auto_match(&state.pool, &app_id, req.account_id, &correlation_id).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// POST /api/treasury/recon/manual-match — create a manual match
#[utoipa::path(
    post, path = "/api/treasury/recon/manual-match", tag = "Reconciliation",
    request_body = ManualMatchRequest,
    responses(
        (status = 201, description = "Match created", body = ReconMatch),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn manual_match(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<ManualMatchRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let correlation_id = correlation(&headers);
    let actor = claims
        .as_ref()
        .map(|Extension(c)| c.user_id.to_string())
        .unwrap_or_else(|| "system".to_string());

    match service::create_manual_match(&state.pool, &app_id, &req, &actor, &correlation_id).await {
        Ok(m) => (StatusCode::CREATED, Json(m)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// GET /api/treasury/recon/matches?account_id=...&include_superseded=false
#[utoipa::path(
    get, path = "/api/treasury/recon/matches", tag = "Reconciliation",
    params(ListMatchesQuery),
    responses(
        (status = 200, description = "Recon matches", body = Vec<ReconMatch>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_matches(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListMatchesQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    match service::list_matches(
        &state.pool,
        &app_id,
        query.account_id,
        query.include_superseded,
    )
    .await
    {
        Ok(matches) => Json(matches).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// Unmatched query params — just account_id
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UnmatchedQuery {
    pub account_id: Uuid,
}

/// GET /api/treasury/recon/unmatched?account_id=...
#[utoipa::path(
    get, path = "/api/treasury/recon/unmatched", tag = "Reconciliation",
    params(UnmatchedQuery),
    responses(
        (status = 200, description = "Unmatched items"),
    ),
    security(("bearer" = [])),
)]
pub async fn list_unmatched(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(query): Query<UnmatchedQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    match service::list_unmatched(&state.pool, &app_id, query.account_id).await {
        Ok(txns) => {
            let (stmt_lines, pay_txns): (Vec<_>, Vec<_>) =
                txns.into_iter().partition(|t| t.statement_id.is_some());

            Json(serde_json::json!({
                "unmatched_statement_lines": stmt_lines.len(),
                "unmatched_payment_transactions": pay_txns.len(),
                "statement_lines": stmt_lines.iter().map(|t| serde_json::json!({
                    "id": t.id,
                    "transaction_date": t.transaction_date,
                    "amount_minor": t.amount_minor,
                    "currency": t.currency,
                    "description": t.description,
                    "reference": t.reference,
                })).collect::<Vec<_>>(),
                "payment_transactions": pay_txns.iter().map(|t| serde_json::json!({
                    "id": t.id,
                    "transaction_date": t.transaction_date,
                    "amount_minor": t.amount_minor,
                    "currency": t.currency,
                    "description": t.description,
                    "reference": t.reference,
                })).collect::<Vec<_>>(),
            }))
            .into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
