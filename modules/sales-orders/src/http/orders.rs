//! HTTP handlers for sales order CRUD and lifecycle operations.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::orders::{
    service, BookOrderRequest, CancelOrderRequest, CreateOrderLineRequest, CreateOrderRequest,
    ListOrdersQuery, UpdateOrderLineRequest, UpdateOrderRequest,
};
use crate::AppState;

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ── Orders ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/so/orders", tag = "SalesOrders",
    request_body = CreateOrderRequest,
    responses(
        (status = 201, description = "Order created", body = crate::domain::orders::SalesOrder),
        (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };
    let created_by = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match service::create_order(&state.pool, &tenant_id, &created_by, req).await {
        Ok(order) => (StatusCode::CREATED, Json(order)).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(ApiError::from(e))).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/so/orders", tag = "SalesOrders",
    responses((status = 200, description = "Order list", body = Vec<crate::domain::orders::SalesOrder>)),
    security(("bearer" = [])),
)]
pub async fn list_orders(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListOrdersQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::list_orders(&state.pool, &tenant_id, &query).await {
        Ok(orders) => Json(orders).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError::from(e))).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/so/orders/{order_id}", tag = "SalesOrders",
    responses(
        (status = 200, description = "Order with lines", body = crate::domain::orders::SalesOrderWithLines),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::get_order_with_lines(&state.pool, &tenant_id, order_id).await {
        Ok(order) => Json(order).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(api_err),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    put, path = "/api/so/orders/{order_id}", tag = "SalesOrders",
    request_body = UpdateOrderRequest,
    responses(
        (status = 200, description = "Order updated", body = crate::domain::orders::SalesOrder),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(order_id): Path<Uuid>,
    Json(req): Json<UpdateOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::update_order(&state.pool, &tenant_id, order_id, req).await {
        Ok(order) => Json(order).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    post, path = "/api/so/orders/{order_id}/book", tag = "SalesOrders",
    request_body = BookOrderRequest,
    responses(
        (status = 200, description = "Order booked", body = crate::domain::orders::SalesOrder),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn book_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(_req): Json<BookOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);
    let inv_url = std::env::var("INVENTORY_BASE_URL").ok();

    match service::book_order(
        &state.pool,
        &tenant_id,
        order_id,
        correlation_id,
        inv_url.as_deref(),
    )
    .await
    {
        Ok(order) => Json(order).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    post, path = "/api/so/orders/{order_id}/cancel", tag = "SalesOrders",
    request_body = CancelOrderRequest,
    responses(
        (status = 200, description = "Order cancelled", body = crate::domain::orders::SalesOrder),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn cancel_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CancelOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);
    let inv_url = std::env::var("INVENTORY_BASE_URL").ok();

    match service::cancel_order(
        &state.pool,
        &tenant_id,
        order_id,
        req,
        correlation_id,
        inv_url.as_deref(),
    )
    .await
    {
        Ok(order) => Json(order).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}

// ── Lines ─────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/so/orders/{order_id}/lines", tag = "SalesOrders",
    request_body = CreateOrderLineRequest,
    responses(
        (status = 201, description = "Line added", body = crate::domain::orders::SalesOrderLine),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CreateOrderLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::add_line(&state.pool, &tenant_id, order_id, req).await {
        Ok(line) => (StatusCode::CREATED, Json(line)).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    put, path = "/api/so/orders/{order_id}/lines/{line_id}", tag = "SalesOrders",
    request_body = UpdateOrderLineRequest,
    responses(
        (status = 200, description = "Line updated", body = crate::domain::orders::SalesOrderLine),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((order_id, line_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateOrderLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::update_line(&state.pool, &tenant_id, order_id, line_id, req).await {
        Ok(line) => Json(line).into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    delete, path = "/api/so/orders/{order_id}/lines/{line_id}", tag = "SalesOrders",
    responses(
        (status = 204, description = "Line removed"),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn remove_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((order_id, line_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(e)).into_response(),
    };

    match service::remove_line(&state.pool, &tenant_id, order_id, line_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let api_err = ApiError::from(e);
            (
                StatusCode::from_u16(api_err.status_code())
                    .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                Json(api_err),
            )
                .into_response()
        }
    }
}
