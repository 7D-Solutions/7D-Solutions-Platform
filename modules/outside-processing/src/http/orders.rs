//! HTTP handlers for OP order CRUD and lifecycle.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::{models::*, repo, state_machine};
use crate::events::{self, *};
use crate::http::tenant::{correlation_from_headers, with_request_id};
use crate::AppState;
use platform_sdk::extract_tenant;

pub async fn create_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateOpOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    let mut tx = match state.pool.begin().await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response(),
    };

    let order_number = match repo::next_op_order_number(&mut tx, &tenant_id).await {
        Ok(n) => n,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let order = match repo::create_order(&mut tx, &tenant_id, &req, &order_number).await {
        Ok(o) => o,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let event_id = Uuid::new_v4();
    let env = events::build_order_created_envelope(
        event_id,
        tenant_id.clone(),
        correlation_id.clone(),
        None,
        OrderCreatedPayload {
            op_order_id: order.op_order_id,
            op_order_number: order.op_order_number.clone(),
            tenant_id: tenant_id.clone(),
            vendor_id: order.vendor_id,
            service_type: order.service_type.clone(),
            work_order_id: order.work_order_id,
            created_at: order.created_at,
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, event_id,
        events::EVENT_ORDER_CREATED, "op_order", &order.op_order_id.to_string(),
        &env, &correlation_id, None,
    ).await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.orders_created.inc();
    (StatusCode::CREATED, Json(order)).into_response()
}

pub async fn get_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::get_order_detail(&state.pool, &tenant_id, order_id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("OP order {} not found", order_id)),
            &tracing_ctx,
        ).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

pub async fn list_orders(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<ListOpOrdersQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_orders(&state.pool, &tenant_id, &q).await {
        Ok(orders) => Json(orders).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

pub async fn update_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(order_id): Path<Uuid>,
    Json(req): Json<UpdateOpOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::update_order(&state.pool, &tenant_id, order_id, &req).await {
        Ok(order) => Json(order).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

pub async fn issue_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<IssueOpOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    let mut tx = match state.pool.begin().await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response(),
    };

    let order = match repo::issue_order(&mut tx, &tenant_id, order_id, req.purchase_order_id).await {
        Ok(o) => o,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let event_id = Uuid::new_v4();
    let env = events::build_order_issued_envelope(
        event_id,
        tenant_id.clone(),
        correlation_id.clone(),
        None,
        OrderIssuedPayload {
            op_order_id: order.op_order_id,
            tenant_id: tenant_id.clone(),
            purchase_order_id: order.purchase_order_id,
            issued_at: Utc::now(),
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, event_id,
        events::EVENT_ORDER_ISSUED, "op_order", &order.op_order_id.to_string(),
        &env, &correlation_id, None,
    ).await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.orders_issued.inc();
    Json(order).into_response()
}

pub async fn cancel_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CancelOpOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    let mut tx = match state.pool.begin().await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response(),
    };

    let order = match repo::lock_order(&mut tx, &tenant_id, order_id).await {
        Ok(o) => o,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let new_status = match state_machine::transition_cancel(&order.status) {
        Ok(s) => s,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let updated = match repo::set_order_status(&mut tx, &tenant_id, order_id, new_status.as_str()).await {
        Ok(o) => o,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let event_id = Uuid::new_v4();
    let env = events::build_order_cancelled_envelope(
        event_id,
        tenant_id.clone(),
        correlation_id.clone(),
        None,
        OrderCancelledPayload {
            op_order_id: order_id,
            tenant_id: tenant_id.clone(),
            reason: req.reason,
            cancelled_at: Utc::now(),
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, event_id,
        events::EVENT_ORDER_CANCELLED, "op_order", &order_id.to_string(),
        &env, &correlation_id, None,
    ).await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.orders_cancelled.inc();
    Json(updated).into_response()
}

pub async fn close_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    let mut tx = match state.pool.begin().await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response(),
    };

    let order = match repo::lock_order(&mut tx, &tenant_id, order_id).await {
        Ok(o) => o,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let new_status = match state_machine::transition_close(&order.status) {
        Ok(s) => s,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let updated = match repo::set_order_status(&mut tx, &tenant_id, order_id, new_status.as_str()).await {
        Ok(o) => o,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let event_id = Uuid::new_v4();
    let env = events::build_order_closed_envelope(
        event_id,
        tenant_id.clone(),
        correlation_id.clone(),
        None,
        OrderClosedPayload {
            op_order_id: order_id,
            tenant_id: tenant_id.clone(),
            closed_at: Utc::now(),
            final_accepted_qty: 0,
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, event_id,
        events::EVENT_ORDER_CLOSED, "op_order", &order_id.to_string(),
        &env, &correlation_id, None,
    ).await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.orders_closed.inc();
    Json(updated).into_response()
}
