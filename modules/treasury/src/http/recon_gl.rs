//! HTTP handlers for GL reconciliation linkage.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::recon::{
    gl_link::{self, LinkToGlRequest, UnmatchedBankTxnGl, UnmatchedGlRequest, UnmatchedGlResult},
    ReconError,
};
use crate::http::recon::ReconErrorBody;
use crate::http::tenant::extract_tenant;
use crate::AppState;

fn recon_error_response(e: ReconError) -> (StatusCode, Json<ReconErrorBody>) {
    match e {
        ReconError::TransactionNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ReconErrorBody::new(
                "transaction_not_found",
                &format!("Bank transaction {} not found", id),
            )),
        ),
        ReconError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ReconErrorBody::new("validation_error", &msg)),
        ),
        ReconError::Database(e) => {
            tracing::error!("Recon GL DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ReconErrorBody::new(
                    "database_error",
                    "Internal database error",
                )),
            )
        }
        other => {
            tracing::error!("Recon GL error: {}", other);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ReconErrorBody::new("internal_error", &other.to_string())),
            )
        }
    }
}

fn tenant_from_claims(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ReconErrorBody>)> {
    extract_tenant(claims)
        .map_err(|(status, Json(e))| (status, Json(ReconErrorBody::new(&e.error, &e.message))))
}

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
pub async fn link_to_gl(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<LinkToGlRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;
    let correlation_id = correlation(&headers);
    let actor = claims
        .as_ref()
        .map(|Extension(c)| c.user_id.to_string())
        .unwrap_or_else(|| "system".to_string());

    let m = gl_link::link_bank_txn_to_gl(&state.pool, &app_id, &req, &actor, &correlation_id)
        .await
        .map_err(recon_error_response)?;

    Ok(Json(serde_json::json!({
        "match_id": m.id,
        "bank_transaction_id": m.bank_transaction_id,
        "gl_entry_id": m.gl_entry_id,
        "status": m.status,
        "match_type": m.match_type,
        "matched_at": m.matched_at,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct UnmatchedBankTxnQuery {
    pub account_id: Uuid,
}

/// GET /api/treasury/recon/gl-unmatched-txns?account_id=... — bank txns not linked to GL
pub async fn unmatched_bank_txns(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<UnmatchedBankTxnQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;

    let txns: Vec<UnmatchedBankTxnGl> =
        gl_link::unmatched_bank_txns_for_gl(&state.pool, &app_id, query.account_id)
            .await
            .map_err(recon_error_response)?;

    Ok(Json(serde_json::json!({
        "count": txns.len(),
        "unmatched_bank_transactions": txns,
    })))
}

/// POST /api/treasury/recon/gl-unmatched-entries — given GL entry IDs, return unlinked ones
pub async fn unmatched_gl_entries(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<UnmatchedGlRequest>,
) -> Result<Json<UnmatchedGlResult>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;

    gl_link::unmatched_gl_entries(&state.pool, &app_id, &req.gl_entry_ids)
        .await
        .map_err(recon_error_response)
        .map(Json)
}
