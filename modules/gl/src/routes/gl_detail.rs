//! GL Detail API Routes
//!
//! Provides HTTP endpoints for querying GL detail reports (journal entries and lines).

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::gl_detail_service::{self, GLDetailResponse};

/// Query parameters for GL detail endpoint
#[derive(Debug, Deserialize)]
pub struct GLDetailQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Accounting period UUID
    pub period_id: Uuid,
    /// Optional account code filter (e.g., "1000")
    pub account_code: Option<String>,
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

/// Handler for GET /api/gl/detail
///
/// Returns paginated GL detail entries (journal entries with lines) for a tenant and period.
/// Supports optional filtering by account_code and currency.
///
/// # Query Parameters
/// - `tenant_id` (required): Tenant identifier
/// - `period_id` (required): Accounting period UUID
/// - `account_code` (optional): Filter by account code
/// - `currency` (optional): Filter by currency (ISO 4217)
/// - `limit` (optional): Page size (1-100, default 50)
/// - `offset` (optional): Pagination offset (default 0)
///
/// # Example
/// ```text
/// GET /api/gl/detail?tenant_id=tenant_123&period_id=uuid&account_code=1000&limit=20&offset=0
/// ```
pub async fn get_gl_detail(
    State(pool): State<Arc<PgPool>>,
    Query(params): Query<GLDetailQuery>,
) -> Result<Json<GLDetailResponse>, GLDetailErrorResponse> {
    // Call service layer
    let response = gl_detail_service::get_gl_detail(
        &pool,
        &params.tenant_id,
        params.period_id,
        params.account_code.as_deref(),
        params.currency.as_deref(),
        params.limit,
        params.offset,
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            gl_detail_service::GLDetailServiceError::InvalidTenantId(_) => {
                StatusCode::BAD_REQUEST
            }
            gl_detail_service::GLDetailServiceError::InvalidPeriod(_) => {
                StatusCode::BAD_REQUEST
            }
            gl_detail_service::GLDetailServiceError::InvalidCurrency(_) => {
                StatusCode::BAD_REQUEST
            }
            gl_detail_service::GLDetailServiceError::InvalidPagination(_) => {
                StatusCode::BAD_REQUEST
            }
            gl_detail_service::GLDetailServiceError::PeriodNotFound { .. } => {
                StatusCode::NOT_FOUND
            }
            gl_detail_service::GLDetailServiceError::Repo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        GLDetailErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct GLDetailErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for GLDetailErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}
