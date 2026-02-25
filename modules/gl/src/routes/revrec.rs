//! Revenue Recognition (Revrec) API Routes — Phase 24a
//!
//! POST /api/gl/revrec/contracts         — Create a revenue contract with obligations
//! POST /api/gl/revrec/schedules         — Generate and persist a recognition schedule
//! POST /api/gl/revrec/recognition-runs  — Execute recognition run for a period
//! POST /api/gl/revrec/amendments        — Amend a contract (mid-cycle schedule versioning)
//!
//! Atomically persists entities + outbox events.
//! Idempotent on contract_id / schedule_id / (schedule, period).

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::repos::revrec_repo::{self, RevrecRepoError};
use crate::revrec::recognition_run::{self, RecognitionRunError};
use crate::revrec::schedule_builder::{generate_schedule, ScheduleBuildError};
use crate::revrec::{
    AllocationChange, ContractCreatedPayload, ContractModifiedPayload, ModificationType,
    PerformanceObligation, RecognitionPattern,
};
use crate::AppState;
use super::auth::extract_tenant;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateContractRequest {
    pub contract_id: Uuid,
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
        RevrecRepoError::DuplicateModification(id) => {
            let body = Json(ErrorResponse {
                error: format!("Modification {} already exists (idempotent)", id),
            });
            (StatusCode::CONFLICT, body).into_response()
        }
        RevrecRepoError::ContractNotFound(id) => {
            let body = Json(ErrorResponse {
                error: format!("Contract {} not found", id),
            });
            (StatusCode::NOT_FOUND, body).into_response()
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
    claims: Option<Extension<VerifiedClaims>>,
    Json(request): Json<CreateContractRequest>,
) -> Result<(StatusCode, Json<CreateContractResponse>), Response> {
    let tenant_id = extract_tenant(&claims).map_err(|(status, msg)| {
        let body = Json(ErrorResponse { error: msg });
        (status, body).into_response()
    })?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let payload = ContractCreatedPayload {
        contract_id: request.contract_id,
        tenant_id: tenant_id.clone(),
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
            tenant_id,
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
    claims: Option<Extension<VerifiedClaims>>,
    Json(request): Json<GenerateScheduleRequest>,
) -> Result<(StatusCode, Json<GenerateScheduleResponse>), Response> {
    let tenant_id = extract_tenant(&claims).map_err(|(status, msg)| {
        let body = Json(ErrorResponse { error: msg });
        (status, body).into_response()
    })?;
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
        &tenant_id,
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

// ============================================================================
// Recognition Run
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RecognitionRunRequest {
    /// Target accounting period (YYYY-MM)
    pub period: String,
    /// Posting date for GL journal entries (YYYY-MM-DD)
    pub posting_date: String,
}

#[derive(Debug, Serialize)]
pub struct RecognitionRunResponse {
    pub period: String,
    pub tenant_id: String,
    pub lines_recognized: usize,
    pub lines_skipped: usize,
    pub total_recognized_minor: i64,
    pub postings: Vec<RecognitionPostingResponse>,
}

#[derive(Debug, Serialize)]
pub struct RecognitionPostingResponse {
    pub run_id: Uuid,
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub journal_entry_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
}

fn map_recognition_run_error(err: RecognitionRunError) -> Response {
    match &err {
        RecognitionRunError::InvalidPostingDate(msg) => {
            let body = Json(ErrorResponse {
                error: format!("Invalid posting_date: {}", msg),
            });
            (StatusCode::BAD_REQUEST, body).into_response()
        }
        RecognitionRunError::Database(_) | RecognitionRunError::Repo(_) => {
            let body = Json(ErrorResponse {
                error: "Internal database error".to_string(),
            });
            (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
        }
    }
}

/// POST /api/gl/revrec/recognition-runs
///
/// Executes a recognition run for a tenant and period.
/// Finds all unrecognized schedule lines due for the period (from latest
/// schedule versions), posts balanced journal entries, and emits outbox events.
///
/// Idempotent: re-running for the same period skips already-recognized lines.
pub async fn run_recognition_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(request): Json<RecognitionRunRequest>,
) -> Result<(StatusCode, Json<RecognitionRunResponse>), Response> {
    let tenant_id = extract_tenant(&claims).map_err(|(status, msg)| {
        let body = Json(ErrorResponse { error: msg });
        (status, body).into_response()
    })?;

    let result = recognition_run::run_recognition(
        &app_state.pool,
        &tenant_id,
        &request.period,
        &request.posting_date,
    )
    .await
    .map_err(map_recognition_run_error)?;

    let postings = result
        .postings
        .iter()
        .map(|p| RecognitionPostingResponse {
            run_id: p.run_id,
            schedule_id: p.schedule_id,
            contract_id: p.contract_id,
            obligation_id: p.obligation_id,
            journal_entry_id: p.journal_entry_id,
            amount_minor: p.amount_minor,
            currency: p.currency.clone(),
        })
        .collect();

    let status = if result.lines_recognized > 0 {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((
        status,
        Json(RecognitionRunResponse {
            period: result.period,
            tenant_id: result.tenant_id,
            lines_recognized: result.lines_recognized,
            lines_skipped: result.lines_skipped,
            total_recognized_minor: result.total_recognized_minor,
            postings,
        }),
    ))
}

// ============================================================================
// Amendment handler
// ============================================================================

#[derive(Debug, Serialize)]
pub struct AmendContractResponse {
    pub modification_id: Uuid,
    pub contract_id: Uuid,
    pub event_id: Uuid,
}

/// POST /api/gl/revrec/amendments
///
/// Record a contract modification. The body is a `ContractModifiedPayload`.
/// Returns 201 on success, 409 if modification_id already exists.
pub async fn amend_contract(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut payload): Json<ContractModifiedPayload>,
) -> Result<(StatusCode, Json<AmendContractResponse>), Response> {
    let tenant_id = extract_tenant(&claims).map_err(|(status, msg)| {
        let body = Json(ErrorResponse { error: msg });
        (status, body).into_response()
    })?;
    // Override client-supplied tenant_id with JWT claims
    payload.tenant_id = tenant_id;

    let event_id = Uuid::new_v4();
    let modification_id = payload.modification_id;
    let contract_id = payload.contract_id;

    revrec_repo::create_amendment(&app_state.pool, event_id, &payload)
        .await
        .map_err(map_revrec_error)?;

    Ok((
        StatusCode::CREATED,
        Json(AmendContractResponse {
            modification_id,
            contract_id,
            event_id,
        }),
    ))
}
