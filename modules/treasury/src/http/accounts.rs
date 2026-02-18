//! HTTP handlers for bank account CRUD.
//!
//! App identity is carried in the `X-App-Id` header.
//! Idempotent creates use the `X-Idempotency-Key` header.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::accounts::{
    service, AccountError, CreateBankAccountRequest, CreateCreditCardAccountRequest,
    TreasuryAccount, UpdateAccountRequest,
};
use crate::AppState;

// ============================================================================
// Helpers
// ============================================================================

fn app_id_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-app-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_app_id", "X-App-Id header is required")),
            )
        })
}

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn idempotency_key_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

fn account_error_response(e: AccountError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        AccountError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "account_not_found",
                &format!("Bank account {} not found", id),
            )),
        ),
        AccountError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        AccountError::IdempotentReplay { .. } => {
            // Caller inspects the error directly; this branch should not be hit
            (
                StatusCode::OK,
                Json(ErrorBody::new("idempotent_replay", "Request already processed")),
            )
        }
        AccountError::Database(e) => {
            tracing::error!("Treasury accounts DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListAccountsQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/treasury/accounts/bank — create a bank account
pub async fn create_bank_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateBankAccountRequest>,
) -> Result<(StatusCode, Json<TreasuryAccount>), (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);
    let idempotency_key = idempotency_key_from_headers(&headers);

    match service::create_bank_account(
        &state.pool,
        &app_id,
        &req,
        idempotency_key.as_deref(),
        correlation_id,
    )
    .await
    {
        Ok(account) => Ok((StatusCode::CREATED, Json(account))),
        Err(AccountError::IdempotentReplay { status_code, body }) => {
            let account: TreasuryAccount = serde_json::from_value(body).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody::new("replay_error", "Failed to deserialize cached response")),
                )
            })?;
            let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(account)))
        }
        Err(e) => Err(account_error_response(e)),
    }
}

/// POST /api/treasury/accounts/credit-card — create a credit card account
pub async fn create_credit_card_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateCreditCardAccountRequest>,
) -> Result<(StatusCode, Json<TreasuryAccount>), (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);
    let idempotency_key = idempotency_key_from_headers(&headers);

    match service::create_credit_card_account(
        &state.pool,
        &app_id,
        &req,
        idempotency_key.as_deref(),
        correlation_id,
    )
    .await
    {
        Ok(account) => Ok((StatusCode::CREATED, Json(account))),
        Err(AccountError::IdempotentReplay { status_code, body }) => {
            let account: TreasuryAccount = serde_json::from_value(body).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody::new("replay_error", "Failed to deserialize cached response")),
                )
            })?;
            let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(account)))
        }
        Err(e) => Err(account_error_response(e)),
    }
}

/// GET /api/treasury/accounts — list accounts for app
pub async fn list_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListAccountsQuery>,
) -> Result<Json<Vec<TreasuryAccount>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let accounts = service::list_accounts(&state.pool, &app_id, query.include_inactive)
        .await
        .map_err(account_error_response)?;

    Ok(Json(accounts))
}

/// GET /api/treasury/accounts/:id — get a single account
pub async fn get_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<TreasuryAccount>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;

    let account = service::get_account(&state.pool, &app_id, id)
        .await
        .map_err(account_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "account_not_found",
                    &format!("Bank account {} not found", id),
                )),
            )
        })?;

    Ok(Json(account))
}

/// PUT /api/treasury/accounts/:id — update account fields
pub async fn update_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAccountRequest>,
) -> Result<Json<TreasuryAccount>, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let account = service::update_account(&state.pool, &app_id, id, &req, correlation_id)
        .await
        .map_err(account_error_response)?;

    Ok(Json(account))
}

/// POST /api/treasury/accounts/:id/deactivate — soft-delete a bank account
pub async fn deactivate_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let app_id = app_id_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);
    let actor = headers
        .get("x-actor-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string();

    service::deactivate_account(&state.pool, &app_id, id, &actor, correlation_id)
        .await
        .map_err(account_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}
