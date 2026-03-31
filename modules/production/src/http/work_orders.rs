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

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::work_orders::{CreateWorkOrderRequest, WorkOrder, WorkOrderRepo},
    AppState,
};

/// POST /api/production/work-orders
#[utoipa::path(
    post,
    path = "/api/production/work-orders",
    tag = "Work Orders",
    request_body = CreateWorkOrderRequest,
    responses(
        (status = 201, description = "Work order created", body = WorkOrder),
        (status = 409, description = "Duplicate order number or correlation", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateWorkOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match WorkOrderRepo::create(&state.pool, &req, &corr, None).await {
        Ok(wo) => (StatusCode::CREATED, Json(wo)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/work-orders/:id/release
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{id}/release",
    tag = "Work Orders",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Work order released", body = WorkOrder),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn release_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match WorkOrderRepo::release(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(wo) => (StatusCode::OK, Json(wo)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/work-orders/:id/close
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{id}/close",
    tag = "Work Orders",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Work order closed", body = WorkOrder),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn close_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match WorkOrderRepo::close(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(wo) => (StatusCode::OK, Json(wo)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/work-orders/:id
#[utoipa::path(
    get,
    path = "/api/production/work-orders/{id}",
    tag = "Work Orders",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Work order details", body = WorkOrder),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_work_order(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match WorkOrderRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(wo)) => (StatusCode::OK, Json(wo)).into_response(),
        Ok(None) => {
            with_request_id(ApiError::not_found("Work order not found"), &tracing_ctx)
                .into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}
