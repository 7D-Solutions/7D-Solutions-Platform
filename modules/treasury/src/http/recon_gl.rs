//! HTTP handlers for GL reconciliation linkage.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::recon::{
    gl_link::{self, LinkToGlRequest, UnmatchedBankTxnGl, UnmatchedGlRequest, UnmatchedGlResult},
    ReconError,
};
use crate::http::recon::ReconErrorBody;
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
                Json(ReconErrorBody::new("database_error", "Internal database error")),
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

fn app_id(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ReconErrorBody>)> {
    headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ReconErrorBody::new("missing_app_id", "X-App-Id header is required")),
            )
        })
}

fn correlation(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn actor(headers: &HeaderMap) -> String {
    headers
        .get("x-actor-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string()
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/treasury/recon/gl-link — link a bank transaction to a GL entry
pub async fn link_to_gl(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<LinkToGlRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = app_id(&headers)?;
    let correlation_id = correlation(&headers);
    let actor = actor(&headers);

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
    headers: HeaderMap,
    Query(query): Query<UnmatchedBankTxnQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = app_id(&headers)?;

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
    headers: HeaderMap,
    Json(req): Json<UnmatchedGlRequest>,
) -> Result<Json<UnmatchedGlResult>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = app_id(&headers)?;

    gl_link::unmatched_gl_entries(&state.pool, &app_id, &req.gl_entry_ids)
        .await
        .map_err(recon_error_response)
        .map(Json)
}
