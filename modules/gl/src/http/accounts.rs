use axum::{extract::State, http::StatusCode, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::auth::with_request_id;
use crate::repos::account_repo::{self, AccountError, AccountType, NormalBalance};
use crate::AppState;
use platform_sdk::extract_tenant;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAccountRequest {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub normal_balance: NormalBalance,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccountResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub account_type: String,
    pub normal_balance: String,
    pub is_active: bool,
}

#[utoipa::path(post, path = "/api/gl/accounts", tag = "Accounts",
    request_body = CreateAccountRequest,
    responses((status = 201, description = "Account created", body = AccountResponse)),
    security(("bearer" = [])))]
pub async fn create_account(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let account = account_repo::create_account(
        &app_state.pool,
        &tenant_id,
        &req.code,
        &req.name,
        req.account_type,
        req.normal_balance,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            AccountError::Conflict { .. } => ApiError::conflict(e.to_string()),
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok((
        StatusCode::CREATED,
        Json(AccountResponse {
            id: account.id,
            tenant_id: account.tenant_id,
            code: account.code,
            name: account.name,
            account_type: format!("{:?}", account.account_type),
            normal_balance: format!("{:?}", account.normal_balance),
            is_active: account.is_active,
        }),
    ))
}
