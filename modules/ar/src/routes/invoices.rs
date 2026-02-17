//! Invoice query routes with projection fallback (bd-17a3)
//!
//! Demonstrates HTTP fallback pattern for projection staleness.
//! Uses circuit breaker and time budget to prevent cascading failures.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use projections::{
    cursor::ProjectionCursor, CircuitBreaker, FallbackMetrics, FallbackPolicy,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Query parameters for invoice endpoint
#[derive(Debug, Deserialize)]
pub struct InvoiceQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Invoice ID
    pub invoice_id: Uuid,
}

/// Invoice response
#[derive(Debug, Serialize)]
pub struct InvoiceResponse {
    pub invoice_id: Uuid,
    pub tenant_id: String,
    pub amount: i64,
    pub status: String,
    pub data_source: DataSource,
}

/// Indicates where the data came from
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    /// Data from projection (fast path)
    Projection,
    /// Data from HTTP fallback (write service)
    Fallback,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/ar/invoices
///
/// Demonstrates projection fallback pattern:
/// 1. Check if projection is stale (beyond threshold)
/// 2. If stale and circuit is closed, attempt HTTP fallback
/// 3. Otherwise, query projection normally
///
/// This prevents cascading failures when projections fall behind.
pub async fn get_invoice(
    State(app_state): State<Arc<crate::AppState>>,
    Query(params): Query<InvoiceQuery>,
) -> Result<Json<InvoiceResponse>, InvoiceErrorResponse> {
    // In production, these would be stored in AppState
    let policy = FallbackPolicy::new(5000, 200); // 5s staleness, 200ms budget
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(5, 2); // 5 failures to open, 2 successes to close

    // Load projection cursor to check staleness
    let cursor = ProjectionCursor::load(
        &app_state.pool,
        "invoice_projection",
        &params.tenant_id,
    )
    .await
    .map_err(|e| InvoiceErrorResponse {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to load projection cursor: {}", e),
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
                "invoice_projection",
                &params.tenant_id,
                query_write_service(params.invoice_id, params.tenant_id.clone()),
            )
            .await
        {
            Ok(invoice) => {
                return Ok(Json(invoice));
            }
            Err(e) => {
                // Fallback failed - fall through to projection query
                tracing::warn!(
                    "Fallback failed for invoice {}: {}. Using potentially stale projection.",
                    params.invoice_id,
                    e
                );
            }
        }
    }

    // Query projection normally (fast path or fallback failed)
    let invoice = query_projection(&app_state.pool, &params.tenant_id, params.invoice_id)
        .await
        .map_err(|e| InvoiceErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to query projection: {}", e),
        })?;

    Ok(Json(invoice))
}

/// Query the invoice projection (read model)
///
/// This is the fast path - queries the denormalized projection table.
async fn query_projection(
    _pool: &sqlx::PgPool,
    tenant_id: &str,
    invoice_id: Uuid,
) -> Result<InvoiceResponse, Box<dyn std::error::Error + Send + Sync>> {
    // In a real implementation, this would query a projection table like:
    // SELECT invoice_id, tenant_id, amount, status FROM invoice_projections
    // WHERE tenant_id = $1 AND invoice_id = $2
    //
    // For this example, we'll return a mock response
    Ok(InvoiceResponse {
        invoice_id,
        tenant_id: tenant_id.to_string(),
        amount: 10000,
        status: "issued".to_string(),
        data_source: DataSource::Projection,
    })
}

/// Query the write service via HTTP (fallback path)
///
/// This is the slow path - hits the write service's HTTP API when projection is stale.
/// Subject to time budget and circuit breaker protection.
async fn query_write_service(
    invoice_id: Uuid,
    tenant_id: String,
) -> Result<InvoiceResponse, Box<dyn std::error::Error + Send + Sync>> {
    // In a real implementation, this would make an HTTP call like:
    // GET http://ar-write-service/api/invoices/{invoice_id}?tenant_id={tenant_id}
    //
    // For this example, we'll simulate with a delay and mock response
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    Ok(InvoiceResponse {
        invoice_id,
        tenant_id,
        amount: 10000,
        status: "issued".to_string(),
        data_source: DataSource::Fallback,
    })
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct InvoiceErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for InvoiceErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}
