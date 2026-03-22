use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::auth::extract_tenant;
use crate::repos::account_repo::{self, AccountError, AccountType, NormalBalance};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub normal_balance: NormalBalance,
}

#[derive(Debug, Serialize)]
pub struct AccountResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub account_type: String,
    pub normal_balance: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// POST /api/gl/accounts
///
/// Create a new chart-of-accounts entry for the authenticated tenant.
/// Returns 201 on success, 409 if the account code already exists.
pub async fn create_account(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountResponse>), AccountErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_status, msg)| AccountErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let account = account_repo::create_account(
        &app_state.pool,
        &tenant_id,
        &req.code,
        &req.name,
        req.account_type,
        req.normal_balance,
    )
    .await
    .map_err(|e| match &e {
        AccountError::Conflict { .. } => AccountErrorResponse {
            status: StatusCode::CONFLICT,
            message: e.to_string(),
        },
        _ => AccountErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        },
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

#[derive(Debug)]
pub struct AccountErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for AccountErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}
