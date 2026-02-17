//! Revenue Recognition (Revrec) API Routes — Phase 24a
//!
//! POST /api/gl/revrec/contracts  — Create a revenue contract with obligations
//! POST /api/gl/revrec/schedules  — Generate and persist a recognition schedule
//!
//! Atomically persists entities + outbox events.
//! Idempotent on contract_id / schedule_id.

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
use crate::revrec::schedule_builder::{generate_schedule, ScheduleBuildError};
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
        RevrecRepoError::DuplicateSchedule(id) => {
            let body = Json(ErrorResponse {
                error: format!("Schedule {} already exists (idempotent)", id),
            });
            (StatusCode::CONFLICT, body).into_response()
        }
        RevrecRepoError::ObligationNotFound(id) => {
            let body = Json(ErrorResponse {
                error: format!("Obligation {} not found", id),
            });
            (StatusCode::NOT_FOUND, body).into_response()
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
        RevrecRepoError::ScheduleSumMismatch { sum, expected } => {
            let body = Json(ErrorResponse {
                error: format!(
                    "Schedule lines sum {} does not match total {}",
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

fn map_schedule_build_error(err: ScheduleBuildError) -> Response {
    let body = Json(ErrorResponse {
        error: err.to_string(),
    });
    (StatusCode::BAD_REQUEST, body).into_response()
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

// ============================================================================
// Schedule Generation
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GenerateScheduleRequest {
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub tenant_id: String,
}

#[derive(Debug, Serialize)]
pub struct GenerateScheduleResponse {
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub version: i32,
    pub lines_count: usize,
    pub total_to_recognize_minor: i64,
    pub first_period: String,
    pub last_period: String,
    pub created_at: DateTime<Utc>,
}

/// POST /api/gl/revrec/schedules
///
/// Generates a recognition schedule for a performance obligation.
/// Fetches the obligation from DB, generates the schedule deterministically,
/// and persists schedule + lines + outbox event atomically.
///
/// Versioned: if an obligation already has a schedule, creates a new version
/// linked to the previous schedule.
pub async fn generate_schedule_handler(
    State(app_state): State<Arc<AppState>>,
    Json(request): Json<GenerateScheduleRequest>,
) -> Result<(StatusCode, Json<GenerateScheduleResponse>), Response> {
    // Fetch obligation from DB
    let obligations = revrec_repo::get_obligations(&app_state.pool, request.contract_id)
        .await
        .map_err(map_revrec_error)?;

    let obligation_row = obligations
        .iter()
        .find(|o| o.obligation_id == request.obligation_id)
        .ok_or_else(|| {
            map_revrec_error(RevrecRepoError::ObligationNotFound(request.obligation_id))
        })?;

    // Reconstruct the PerformanceObligation from the DB row
    let recognition_pattern: RecognitionPattern =
        serde_json::from_value(obligation_row.recognition_pattern.clone()).map_err(|e| {
            map_revrec_error(RevrecRepoError::Serialization(format!(
                "Invalid recognition_pattern in DB: {}",
                e
            )))
        })?;

    let obligation = PerformanceObligation {
        obligation_id: obligation_row.obligation_id,
        name: obligation_row.name.clone(),
        description: obligation_row.description.clone(),
        allocated_amount_minor: obligation_row.allocated_amount_minor,
        recognition_pattern,
        satisfaction_start: obligation_row.satisfaction_start.format("%Y-%m-%d").to_string(),
        satisfaction_end: obligation_row
            .satisfaction_end
            .map(|d| d.format("%Y-%m-%d").to_string()),
    };

    // Fetch contract for currency
    let contract = revrec_repo::get_contract(&app_state.pool, request.contract_id)
        .await
        .map_err(map_revrec_error)?
        .ok_or_else(|| {
            let body = Json(ErrorResponse {
                error: format!("Contract {} not found", request.contract_id),
            });
            (StatusCode::NOT_FOUND, body).into_response()
        })?;

    let schedule_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    // Generate schedule deterministically
    let payload = generate_schedule(
        schedule_id,
        request.contract_id,
        &obligation,
        &request.tenant_id,
        &contract.currency,
        now,
    )
    .map_err(map_schedule_build_error)?;

    let lines_count = payload.lines.len();
    let first_period = payload.first_period.clone();
    let last_period = payload.last_period.clone();
    let total = payload.total_to_recognize_minor;

    // Persist atomically
    revrec_repo::create_schedule(&app_state.pool, event_id, &payload)
        .await
        .map_err(map_revrec_error)?;

    // Get the version that was assigned
    let schedule_row = revrec_repo::get_schedule(&app_state.pool, schedule_id)
        .await
        .map_err(map_revrec_error)?
        .expect("Schedule just created must exist");

    Ok((
        StatusCode::CREATED,
        Json(GenerateScheduleResponse {
            schedule_id,
            contract_id: request.contract_id,
            obligation_id: request.obligation_id,
            version: schedule_row.version,
            lines_count,
            total_to_recognize_minor: total,
            first_period,
            last_period,
            created_at: now,
        }),
    ))
}
