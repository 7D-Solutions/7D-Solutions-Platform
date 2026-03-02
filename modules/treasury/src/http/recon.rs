//! HTTP handlers for bank reconciliation.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::recon::{
    models::{AutoMatchRequest, AutoMatchResult, ListMatchesQuery, ManualMatchRequest, ReconMatch},
    service, ReconError,
};
use crate::http::tenant::extract_tenant;
use crate::AppState;

// ============================================================================
// Error body
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ReconErrorBody {
    pub error: String,
    pub message: String,
}

impl ReconErrorBody {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

fn recon_error_response(e: ReconError) -> (StatusCode, Json<ReconErrorBody>) {
    match e {
        ReconError::StatementLineNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ReconErrorBody::new(
                "statement_line_not_found",
                &format!("Statement line {} not found", id),
            )),
        ),
        ReconError::TransactionNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ReconErrorBody::new(
                "transaction_not_found",
                &format!("Bank transaction {} not found", id),
            )),
        ),
        ReconError::MatchNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ReconErrorBody::new(
                "match_not_found",
                &format!("Recon match {} not found", id),
            )),
        ),
        ReconError::AmountMismatch {
            stmt_amount,
            txn_amount,
        } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ReconErrorBody::new(
                "amount_mismatch",
                &format!(
                    "Statement amount {} does not match transaction amount {}",
                    stmt_amount, txn_amount
                ),
            )),
        ),
        ReconError::CurrencyMismatch {
            stmt_currency,
            txn_currency,
        } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ReconErrorBody::new(
                "currency_mismatch",
                &format!("Currency mismatch: {} vs {}", stmt_currency, txn_currency),
            )),
        ),
        ReconError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ReconErrorBody::new("validation_error", &msg)),
        ),
        ReconError::Database(e) => {
            tracing::error!("Recon DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ReconErrorBody::new(
                    "database_error",
                    "Internal database error",
                )),
            )
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

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

/// POST /api/treasury/recon/auto-match — run auto-match for an account
pub async fn auto_match(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<AutoMatchRequest>,
) -> Result<Json<AutoMatchResult>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;
    let correlation_id = correlation(&headers);

    service::run_auto_match(&state.pool, &app_id, req.account_id, &correlation_id)
        .await
        .map(Json)
        .map_err(recon_error_response)
}

/// POST /api/treasury/recon/manual-match — create a manual match
pub async fn manual_match(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<ManualMatchRequest>,
) -> Result<(StatusCode, Json<ReconMatch>), (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;
    let correlation_id = correlation(&headers);
    let actor = claims
        .as_ref()
        .map(|Extension(c)| c.user_id.to_string())
        .unwrap_or_else(|| "system".to_string());

    service::create_manual_match(&state.pool, &app_id, &req, &actor, &correlation_id)
        .await
        .map(|m| (StatusCode::CREATED, Json(m)))
        .map_err(recon_error_response)
}

/// GET /api/treasury/recon/matches?account_id=...&include_superseded=false
pub async fn list_matches(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListMatchesQuery>,
) -> Result<Json<Vec<ReconMatch>>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;

    service::list_matches(
        &state.pool,
        &app_id,
        query.account_id,
        query.include_superseded,
    )
    .await
    .map(Json)
    .map_err(recon_error_response)
}

/// Unmatched query params — just account_id
#[derive(Debug, serde::Deserialize)]
pub struct UnmatchedQuery {
    pub account_id: Uuid,
}

/// GET /api/treasury/recon/unmatched?account_id=...
pub async fn list_unmatched(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<UnmatchedQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ReconErrorBody>)> {
    let app_id = tenant_from_claims(&claims)?;

    let txns = service::list_unmatched(&state.pool, &app_id, query.account_id)
        .await
        .map_err(recon_error_response)?;

    // Split into statement lines vs payment txns for clarity
    let (stmt_lines, pay_txns): (Vec<_>, Vec<_>) =
        txns.into_iter().partition(|t| t.statement_id.is_some());

    Ok(Json(serde_json::json!({
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
    })))
}
