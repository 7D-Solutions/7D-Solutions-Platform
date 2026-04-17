use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::cost_tracking::{
        CostPosting, CostRepo, CostSummary, CostTrackingError, PostCostRequest, PostingCategory,
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ManualPostCostRequest {
    pub operation_id: Option<Uuid>,
    pub posting_category: PostingCategory,
    pub amount_cents: i64,
    pub quantity: Option<f64>,
}

// ============================================================================
// POST /api/production/work-orders/:id/cost-postings
// ============================================================================

/// POST /api/production/work-orders/{id}/cost-postings
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{id}/cost-postings",
    tag = "Cost Tracking",
    params(("id" = Uuid, Path, description = "Work order ID")),
    request_body = ManualPostCostRequest,
    responses(
        (status = 201, description = "Cost posting created", body = CostPosting),
        (status = 409, description = "Duplicate source event", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_cost_posting(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(work_order_id): Path<Uuid>,
    Json(body): Json<ManualPostCostRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let posted_by = claims
        .as_ref()
        .map(|Extension(c)| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let corr = Uuid::new_v4().to_string();
    let req = PostCostRequest {
        work_order_id,
        operation_id: body.operation_id,
        posting_category: body.posting_category,
        amount_cents: body.amount_cents,
        quantity: body.quantity,
        source_event_id: None,
        posted_by,
    };

    match CostRepo::post_cost(&state.pool, &req, &tenant_id, &corr, None).await {
        Ok(posting) => (StatusCode::CREATED, Json(posting)).into_response(),
        Err(CostTrackingError::DuplicateSourceEvent) => with_request_id(
            ApiError::conflict("A cost posting with this source event already exists"),
            &tracing_ctx,
        )
        .into_response(),
        Err(CostTrackingError::WorkOrderNotFound) => {
            with_request_id(ApiError::not_found("Work order not found"), &tracing_ctx)
                .into_response()
        }
        Err(CostTrackingError::Database(e)) => {
            tracing::error!(error = %e, "cost posting database error");
            with_request_id(ApiError::internal("Internal server error"), &tracing_ctx)
                .into_response()
        }
    }
}

// ============================================================================
// GET /api/production/work-orders/:id/cost-summary
// ============================================================================

/// GET /api/production/work-orders/{id}/cost-summary
#[utoipa::path(
    get,
    path = "/api/production/work-orders/{id}/cost-summary",
    tag = "Cost Tracking",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Cost summary", body = CostSummary),
        (status = 404, description = "No cost data for this work order", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_cost_summary(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(work_order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match CostRepo::get_summary(&state.pool, work_order_id, &tenant_id).await {
        Ok(Some(summary)) => Json(summary).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found("No cost summary found for this work order"),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "cost summary database error");
            with_request_id(ApiError::internal("Internal server error"), &tracing_ctx)
                .into_response()
        }
    }
}

// ============================================================================
// GET /api/production/work-orders/:id/cost-postings
// ============================================================================

/// GET /api/production/work-orders/{id}/cost-postings
#[utoipa::path(
    get,
    path = "/api/production/work-orders/{id}/cost-postings",
    tag = "Cost Tracking",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "List of cost postings", body = Vec<CostPosting>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_cost_postings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(work_order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match CostRepo::list_postings(&state.pool, work_order_id, &tenant_id).await {
        Ok(postings) => Json(postings).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list cost postings database error");
            with_request_id(ApiError::internal("Internal server error"), &tracing_ctx)
                .into_response()
        }
    }
}
