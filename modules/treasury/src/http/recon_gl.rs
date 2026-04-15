//! HTTP handlers for GL reconciliation linkage.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::recon::gl_link::{
    self, GlLinkResponse, LinkToGlRequest, UnmatchedBankTxnGl, UnmatchedGlRequest,
    UnmatchedGlResult,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

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

/// POST /api/treasury/recon/gl-link — link a bank transaction to a GL entry
#[utoipa::path(
    post, path = "/api/treasury/recon/gl-link", tag = "GL Reconciliation",
    request_body = LinkToGlRequest,
    responses(
        (status = 200, description = "Link created", body = GlLinkResponse),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn link_to_gl(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<LinkToGlRequest>,
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

    match gl_link::link_bank_txn_to_gl(&state.pool, &app_id, &req, &actor, &correlation_id).await {
        Ok(m) => Json(GlLinkResponse {
            match_id: m.id,
            bank_transaction_id: m.bank_transaction_id,
            gl_entry_id: m.gl_entry_id,
            status: m.status,
            match_type: m.match_type,
            matched_at: m.matched_at,
        })
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UnmatchedBankTxnQuery {
    pub account_id: Uuid,
}

/// Unmatched bank txns response envelope
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct UnmatchedBankTxnsResponse {
    pub count: usize,
    pub unmatched_bank_transactions: Vec<UnmatchedBankTxnGl>,
}

/// GET /api/treasury/recon/gl-unmatched-txns?account_id=... — bank txns not linked to GL
#[utoipa::path(
    get, path = "/api/treasury/recon/gl-unmatched-txns", tag = "GL Reconciliation",
    params(UnmatchedBankTxnQuery),
    responses(
        (status = 200, description = "Unmatched bank transactions", body = UnmatchedBankTxnsResponse),
    ),
    security(("bearer" = [])),
)]
pub async fn unmatched_bank_txns(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(query): Query<UnmatchedBankTxnQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    match gl_link::unmatched_bank_txns_for_gl(&state.pool, &app_id, query.account_id).await {
        Ok(txns) => Json(UnmatchedBankTxnsResponse {
            count: txns.len(),
            unmatched_bank_transactions: txns,
        })
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// POST /api/treasury/recon/gl-unmatched-entries — given GL entry IDs, return unlinked ones
#[utoipa::path(
    post, path = "/api/treasury/recon/gl-unmatched-entries", tag = "GL Reconciliation",
    request_body = UnmatchedGlRequest,
    responses(
        (status = 200, description = "Unmatched GL entries", body = UnmatchedGlResult),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn unmatched_gl_entries(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<UnmatchedGlRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    match gl_link::unmatched_gl_entries(&state.pool, &app_id, &req.gl_entry_ids).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
