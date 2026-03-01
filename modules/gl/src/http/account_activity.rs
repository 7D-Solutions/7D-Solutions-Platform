//! Account Activity API Routes
//!
//! Provides HTTP endpoints for querying account activity (journal lines for a specific account).

use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::auth::extract_tenant;
use crate::services::account_activity_service::{self, AccountActivityResponse};

/// Query parameters for account activity endpoint
#[derive(Debug, Deserialize)]
pub struct AccountActivityQuery {
    /// Optional accounting period UUID (required if start_date/end_date not provided)
    pub period_id: Option<Uuid>,
    /// Optional start date (ISO 8601, required if period_id not provided)
    pub start_date: Option<DateTime<Utc>>,
    /// Optional end date (ISO 8601, required if period_id not provided)
    pub end_date: Option<DateTime<Utc>>,
    /// Optional currency filter (ISO 4217, e.g., "USD")
    pub currency: Option<String>,
    /// Page size (1-100, default 50)
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Pagination offset (default 0)
    #[serde(default)]
    pub offset: i64,
}

/// Default limit for pagination
fn default_limit() -> i64 {
    50
}

/// Error response
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/gl/accounts/{account_code}/activity
///
/// Returns paginated journal lines for a specific account within a period or date range.
///
/// # Path Parameters
/// - `account_code` (required): Chart of Accounts code (e.g., "1000")
///
/// # Query Parameters
/// - `period_id` (optional): Accounting period UUID (mutually exclusive with date range)
/// - `start_date` (optional): Start date (ISO 8601, required if no period_id)
/// - `end_date` (optional): End date (ISO 8601, required if no period_id)
/// - `currency` (optional): Filter by currency (ISO 4217)
/// - `limit` (optional): Page size (1-100, default 50)
/// - `offset` (optional): Pagination offset (default 0)
///
/// Tenant identity is derived from JWT claims (VerifiedClaims).
///
/// # Example
/// ```text
/// GET /api/gl/accounts/1000/activity?period_id=uuid&limit=20&offset=0
/// ```
pub async fn get_account_activity(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(account_code): Path<String>,
    Query(params): Query<AccountActivityQuery>,
) -> Result<Json<AccountActivityResponse>, AccountActivityErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| AccountActivityErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    // Call service layer
    let response = account_activity_service::get_account_activity(
        &app_state.pool,
        &tenant_id,
        &account_code,
        params.period_id,
        params.start_date,
        params.end_date,
        params.currency.as_deref(),
        params.limit,
        params.offset,
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            account_activity_service::AccountActivityServiceError::InvalidTenantId(_) => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::InvalidAccountCode(_) => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::InvalidPeriod(_) => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::InvalidCurrency(_) => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::InvalidPagination(_) => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::InvalidDateRange(_) => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::MissingDateFilter => {
                StatusCode::BAD_REQUEST
            }
            account_activity_service::AccountActivityServiceError::PeriodNotFound { .. } => {
                StatusCode::NOT_FOUND
            }
            account_activity_service::AccountActivityServiceError::Repo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        AccountActivityErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct AccountActivityErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for AccountActivityErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}
