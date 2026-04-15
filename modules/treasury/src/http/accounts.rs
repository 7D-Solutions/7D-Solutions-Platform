//! HTTP handlers for bank account CRUD.
//!
//! Tenant identity is derived from JWT claims via [`VerifiedClaims`].
//! All operations are tenant-scoped; cross-tenant access is impossible.
//! Idempotent creates use the `X-Idempotency-Key` header.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::accounts::{
    service, AccountError, CreateBankAccountRequest, CreateCreditCardAccountRequest,
    TreasuryAccount, UpdateAccountRequest,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Helpers
// ============================================================================

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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListAccountsQuery {
    #[serde(default)]
    pub include_inactive: bool,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/treasury/accounts/bank — create a bank account
#[utoipa::path(
    post, path = "/api/treasury/accounts/bank", tag = "Accounts",
    request_body = CreateBankAccountRequest,
    responses(
        (status = 201, description = "Account created", body = TreasuryAccount),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn create_bank_account(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateBankAccountRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
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
        Ok(account) => (StatusCode::CREATED, Json(account)).into_response(),
        Err(AccountError::IdempotentReplay { status_code, body }) => {
            match serde_json::from_value::<TreasuryAccount>(body) {
                Ok(account) => {
                    let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
                    (status, Json(account)).into_response()
                }
                Err(_) => with_request_id(
                    ApiError::internal("Failed to deserialize cached response"),
                    &ctx,
                )
                .into_response(),
            }
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// POST /api/treasury/accounts/credit-card — create a credit card account
#[utoipa::path(
    post, path = "/api/treasury/accounts/credit-card", tag = "Accounts",
    request_body = CreateCreditCardAccountRequest,
    responses(
        (status = 201, description = "Account created", body = TreasuryAccount),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn create_credit_card_account(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateCreditCardAccountRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
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
        Ok(account) => (StatusCode::CREATED, Json(account)).into_response(),
        Err(AccountError::IdempotentReplay { status_code, body }) => {
            match serde_json::from_value::<TreasuryAccount>(body) {
                Ok(account) => {
                    let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
                    (status, Json(account)).into_response()
                }
                Err(_) => with_request_id(
                    ApiError::internal("Failed to deserialize cached response"),
                    &ctx,
                )
                .into_response(),
            }
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// GET /api/treasury/accounts — list accounts for tenant
#[utoipa::path(
    get, path = "/api/treasury/accounts", tag = "Accounts",
    params(ListAccountsQuery),
    responses(
        (status = 200, description = "Paginated accounts", body = PaginatedResponse<TreasuryAccount>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_accounts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListAccountsQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);

    let total = match service::count_accounts(&state.pool, &app_id, query.include_inactive).await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(e), &ctx).into_response(),
    };
    match service::list_accounts_paginated(
        &state.pool,
        &app_id,
        query.include_inactive,
        page_size,
        (page - 1) * page_size,
    )
    .await
    {
        Ok(accounts) => {
            Json(PaginatedResponse::new(accounts, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// GET /api/treasury/accounts/:id — get a single account
#[utoipa::path(
    get, path = "/api/treasury/accounts/{id}", tag = "Accounts",
    params(("id" = Uuid, Path, description = "Account ID")),
    responses(
        (status = 200, description = "Account found", body = TreasuryAccount),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_account(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    match service::get_account(&state.pool, &app_id, id).await {
        Ok(Some(account)) => Json(account).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Treasury account {} not found", id)),
            &ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// PUT /api/treasury/accounts/:id — update account fields
#[utoipa::path(
    put, path = "/api/treasury/accounts/{id}", tag = "Accounts",
    params(("id" = Uuid, Path, description = "Account ID")),
    request_body = UpdateAccountRequest,
    responses(
        (status = 200, description = "Account updated", body = TreasuryAccount),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn update_account(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAccountRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::update_account(&state.pool, &app_id, id, &req, correlation_id).await {
        Ok(account) => Json(account).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

/// POST /api/treasury/accounts/:id/deactivate — soft-delete a bank account
#[utoipa::path(
    post, path = "/api/treasury/accounts/{id}/deactivate", tag = "Accounts",
    params(("id" = Uuid, Path, description = "Account ID")),
    responses(
        (status = 204, description = "Account deactivated"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["TREASURY_MUTATE"])),
)]
pub async fn deactivate_account(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);
    let actor = claims
        .as_ref()
        .map(|Extension(c)| c.user_id.to_string())
        .unwrap_or_else(|| "system".to_string());

    match service::deactivate_account(&state.pool, &app_id, id, &actor, correlation_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
