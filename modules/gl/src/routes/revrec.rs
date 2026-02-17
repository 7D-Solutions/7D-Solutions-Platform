//! Revenue Recognition (Revrec) API Routes — Phase 24a
//!
//! POST /api/gl/revrec/contracts — Create a revenue contract with obligations
//!
//! Atomically persists contract + obligations + outbox event.
//! Idempotent on contract_id.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::repos::revrec_repo::{self, RevrecRepoError};
use crate::revrec::{ContractCreatedPayload, PerformanceObligation, RecognitionPattern};
use crate::AppState;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateContractRequest {
    pub contract_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub contract_name: String,
    pub contract_start: String,
    pub contract_end: Option<String>,
    pub total_transaction_price_minor: i64,
    pub currency: String,
    pub performance_obligations: Vec<PerformanceObligation>,
    pub external_contract_ref: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateContractResponse {
    pub contract_id: Uuid,
    pub tenant_id: String,
    pub obligations_count: usize,
    pub total_transaction_price_minor: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ============================================================================
// Error mapping
// ============================================================================

fn map_revrec_error(err: RevrecRepoError) -> Response {
    match &err {
        RevrecRepoError::DuplicateContract(id) => {
            let body = Json(ErrorResponse {
                error: format!("Contract {} already exists (idempotent)", id),
            });
            (StatusCode::CONFLICT, body).into_response()
        }
        RevrecRepoError::AllocationMismatch { sum, expected } => {
            let body = Json(ErrorResponse {
                error: format!(
                    "Allocation sum mismatch: obligations sum to {}, expected {}",
                    sum, expected
                ),
            });
            (StatusCode::BAD_REQUEST, body).into_response()
        }
        RevrecRepoError::Serialization(msg) => {
            let body = Json(ErrorResponse {
                error: format!("Invalid input: {}", msg),
            });
            (StatusCode::BAD_REQUEST, body).into_response()
        }
        RevrecRepoError::Database(_) => {
            let body = Json(ErrorResponse {
                error: "Internal database error".to_string(),
            });
            (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
        }
    }
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/gl/revrec/contracts
///
/// Creates a revenue contract with performance obligations.
/// Atomic: contract + obligations + outbox event in a single transaction.
/// Idempotent: returns 409 CONFLICT if contract_id already exists.
pub async fn create_contract(
    State(app_state): State<Arc<AppState>>,
    Json(request): Json<CreateContractRequest>,
) -> Result<(StatusCode, Json<CreateContractResponse>), Response> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let payload = ContractCreatedPayload {
        contract_id: request.contract_id,
        tenant_id: request.tenant_id.clone(),
        customer_id: request.customer_id.clone(),
        contract_name: request.contract_name.clone(),
        contract_start: request.contract_start.clone(),
        contract_end: request.contract_end.clone(),
        total_transaction_price_minor: request.total_transaction_price_minor,
        currency: request.currency.clone(),
        performance_obligations: request.performance_obligations.clone(),
        external_contract_ref: request.external_contract_ref.clone(),
        created_at: now,
    };

    let obligations_count = payload.performance_obligations.len();

    revrec_repo::create_contract(&app_state.pool, event_id, &payload)
        .await
        .map_err(map_revrec_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateContractResponse {
            contract_id: request.contract_id,
            tenant_id: request.tenant_id,
            obligations_count,
            total_transaction_price_minor: request.total_transaction_price_minor,
            created_at: now,
        }),
    ))
}
