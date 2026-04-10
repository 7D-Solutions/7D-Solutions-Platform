use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::work_orders::{CreateWorkOrderRequest, WorkOrder, WorkOrderRepo, WorkOrderResponse},
    AppState,
};

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

/// Query parameters for `GET /api/production/work-orders`.
///
/// When `ids` is absent the endpoint returns a paginated list.
/// When `ids` is present (comma-separated UUIDs, max 50) it returns the
/// matching work orders as a flat array, optionally with nested sub-collections
/// controlled by `include`.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct WorkOrderListQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    /// Comma-separated work-order UUIDs for batch fetch (max 50).
    pub ids: Option<String>,
    /// Comma-separated sub-collections to embed: `operations`, `time_entries`.
    pub include: Option<String>,
}

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

/// GET /api/production/work-orders
///
/// **Paginated list** (default): returns a paginated list of work orders when
/// `ids` is absent.
///
/// **Batch fetch**: when `?ids=uuid1,uuid2,...` is present (max 50 IDs) returns
/// a flat array of matching work orders.  Use `?include=operations` or
/// `?include=operations,time_entries` to embed nested sub-collections in a
/// single round-trip.
#[utoipa::path(
    get,
    path = "/api/production/work-orders",
    tag = "Work Orders",
    params(WorkOrderListQuery),
    responses(
        (status = 200, description = "Paginated work order list or batch array", body = PaginatedResponse<WorkOrderResponse>),
        (status = 400, description = "ids is empty or exceeds 50", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_work_orders(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<WorkOrderListQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    if let Some(ids_str) = q.ids {
        // ── Batch mode ──────────────────────────────────────────────────────
        let ids_str = ids_str.trim().to_string();
        if ids_str.is_empty() {
            return with_request_id(
                ApiError::bad_request("ids must not be empty"),
                &tracing_ctx,
            )
            .into_response();
        }
        let ids: Result<Vec<Uuid>, _> = ids_str
            .split(',')
            .map(|s| s.trim().parse::<Uuid>())
            .collect();
        let ids = match ids {
            Ok(v) => v,
            Err(_) => {
                return with_request_id(
                    ApiError::bad_request("ids contains an invalid UUID"),
                    &tracing_ctx,
                )
                .into_response()
            }
        };
        if ids.is_empty() {
            return with_request_id(
                ApiError::bad_request("ids must contain at least one ID"),
                &tracing_ctx,
            )
            .into_response();
        }
        if ids.len() > 50 {
            return with_request_id(
                ApiError::bad_request("ids exceeds maximum of 50"),
                &tracing_ctx,
            )
            .into_response();
        }

        let include = q.include.as_deref().unwrap_or("");
        let include_ops = include.contains("operations");
        let include_te = include.contains("time_entries");

        match WorkOrderRepo::fetch_batch(&state.pool, &ids, &tenant_id, include_ops, include_te)
            .await
        {
            Ok(items) => (StatusCode::OK, Json(items)).into_response(),
            Err(e) => {
                let api_err: ApiError = e.into();
                with_request_id(api_err, &tracing_ctx).into_response()
            }
        }
    } else {
        // ── Paginated list mode (existing behaviour) ─────────────────────────
        let page = q.page.max(1);
        let page_size = q.page_size.clamp(1, 200);
        match WorkOrderRepo::list_with_derived(&state.pool, &tenant_id, page, page_size).await {
            Ok((items, total)) => {
                let resp = PaginatedResponse::new(items, page, page_size, total);
                (StatusCode::OK, Json(resp)).into_response()
            }
            Err(e) => {
                let api_err: ApiError = e.into();
                with_request_id(api_err, &tracing_ctx).into_response()
            }
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
        (status = 200, description = "Work order details", body = WorkOrderResponse),
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
    match WorkOrderRepo::find_by_id_with_derived(&state.pool, id, &tenant_id).await {
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
