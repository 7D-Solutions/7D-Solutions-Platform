//! Payment query routes with projection fallback (bd-17a3)
//!
//! Demonstrates HTTP fallback pattern for projection staleness.
//! Uses circuit breaker and time budget to prevent cascading failures.

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use projections::{cursor::ProjectionCursor, CircuitBreaker, FallbackMetrics, FallbackPolicy};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::http::checkout_sessions::extract_tenant;

fn with_request_id(err: ApiError, ctx: &Option<Extension<TracingContext>>) -> ApiError {
    match ctx {
        Some(Extension(c)) => {
            if let Some(tid) = &c.trace_id {
                err.with_request_id(tid.clone())
            } else {
                err
            }
        }
        None => err,
    }
}

/// Query parameters for payment endpoint
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PaymentQuery {
    /// Payment ID
    pub payment_id: Uuid,
}

/// Payment response
#[derive(Debug, Serialize, ToSchema)]
pub struct PaymentResponse {
    pub payment_id: Uuid,
    pub tenant_id: String,
    pub amount: i64,
    pub status: String,
    pub data_source: DataSource,
}

/// Indicates where the data came from
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    /// Data from projection (fast path)
    Projection,
    /// Data from HTTP fallback (write service)
    Fallback,
}

/// Handler for GET /api/payments/payments
///
/// Demonstrates projection fallback pattern:
/// 1. Check if projection is stale (beyond threshold)
/// 2. If stale and circuit is closed, attempt HTTP fallback
/// 3. Otherwise, query projection normally
///
/// This prevents cascading failures when projections fall behind.
#[utoipa::path(
    get,
    path = "/api/payments/payments",
    tag = "Payments",
    params(PaymentQuery),
    responses(
        (status = 200, description = "Payment details", body = PaymentResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["PAYMENTS_MUTATE"]))
)]
pub async fn get_payment(
    State(app_state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<PaymentQuery>,
) -> Result<Json<PaymentResponse>, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &tracing_ctx))?;

    // In production, these would be stored in AppState
    let policy = FallbackPolicy::new(5000, 200); // 5s staleness, 200ms budget
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(5, 2); // 5 failures to open, 2 successes to close

    // Load projection cursor to check staleness
    let cursor = ProjectionCursor::load(&app_state.pool, "payment_projection", &tenant_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to load projection cursor: {}", e);
            with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            )
        })?;

    // Check if projection is stale and fallback is possible
    let use_fallback = cursor
        .as_ref()
        .map(|c| policy.is_stale(c) && circuit.is_closed())
        .unwrap_or(false);

    if use_fallback {
        // Attempt HTTP fallback with circuit breaker and budget
        match policy
            .execute_with_budget(
                &metrics,
                &circuit,
                "payment_projection",
                &tenant_id,
                query_write_service(params.payment_id, tenant_id.clone()),
            )
            .await
        {
            Ok(payment) => {
                return Ok(Json(payment));
            }
            Err(e) => {
                // Fallback failed - fall through to projection query
                tracing::warn!(
                    "Fallback failed for payment {}: {}. Using potentially stale projection.",
                    params.payment_id,
                    e
                );
            }
        }
    }

    // Query projection normally (fast path or fallback failed)
    let payment = query_projection(&app_state.pool, &tenant_id, params.payment_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to query payment projection: {}", e);
            with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            )
        })?;

    Ok(Json(payment))
}

/// Query the payment projection (read model)
///
/// This is the fast path - queries the denormalized projection table.
async fn query_projection(
    _pool: &sqlx::PgPool,
    tenant_id: &str,
    payment_id: Uuid,
) -> Result<PaymentResponse, Box<dyn std::error::Error + Send + Sync>> {
    // In a real implementation, this would query a projection table like:
    // SELECT payment_id, tenant_id, amount, status FROM payment_projections
    // WHERE tenant_id = $1 AND payment_id = $2
    //
    // For this example, we'll return a mock response
    Ok(PaymentResponse {
        payment_id,
        tenant_id: tenant_id.to_string(),
        amount: 5000,
        status: "completed".to_string(),
        data_source: DataSource::Projection,
    })
}

/// Query the write service via HTTP (fallback path)
///
/// This is the slow path - hits the write service's HTTP API when projection is stale.
/// Subject to time budget and circuit breaker protection.
async fn query_write_service(
    payment_id: Uuid,
    tenant_id: String,
) -> Result<PaymentResponse, Box<dyn std::error::Error + Send + Sync>> {
    // In a real implementation, this would make an HTTP call like:
    // GET http://payments-write-service/api/payments/{payment_id}?tenant_id={tenant_id}
    //
    // For this example, we'll simulate with a delay and mock response
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    Ok(PaymentResponse {
        payment_id,
        tenant_id,
        amount: 5000,
        status: "completed".to_string(),
        data_source: DataSource::Fallback,
    })
}
