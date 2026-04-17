//! HTTP handlers for return events.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
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

pub async fn create_return_event(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CreateReturnEventRequest>,
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

    let (order, sum_shipped, sum_received) =
        match repo::lock_order_for_quantity_check(&mut tx, &tenant_id, order_id).await {
            Ok(v) => v,
            Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
        };

    // Must have at least one ship event before a return
    if sum_shipped == 0 {
        return with_request_id(
            ApiError::new(422, "no_ship_events", "Cannot record a return before any shipment"),
            &tracing_ctx,
        ).into_response();
    }

    // Validate state
    let new_status = match state_machine::transition_on_return_event(&order.status) {
        Ok(s) => s,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    // Return quantity bound
    let new_total = sum_received + req.quantity_received as i64;
    if new_total > sum_shipped {
        return with_request_id(
            ApiError::new(422, "quantity_exceeded",
                format!("Return quantity {} would exceed total shipped {} (already received: {})",
                    req.quantity_received, sum_shipped, sum_received)),
            &tracing_ctx,
        ).into_response();
    }

    let ret_event = match repo::create_return_event_tx(&mut tx, &tenant_id, order_id, &req).await {
        Ok(e) => e,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    if let Err(e) = repo::set_order_status(&mut tx, &tenant_id, order_id, new_status.as_str()).await {
        return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
    }

    let event_id = Uuid::new_v4();
    let env = events::build_returned_envelope(
        event_id, tenant_id.clone(), correlation_id.clone(), None,
        ReturnedPayload {
            op_order_id: order_id,
            return_event_id: ret_event.id,
            tenant_id: tenant_id.clone(),
            quantity_received: req.quantity_received,
            condition: req.condition.as_str().to_string(),
            received_date: req.received_date,
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, event_id,
        events::EVENT_RETURNED, "op_order", &order_id.to_string(),
        &env, &correlation_id, None,
    ).await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.return_events_recorded.inc();
    (StatusCode::CREATED, Json(ret_event)).into_response()
}

pub async fn list_return_events(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_return_events(&state.pool, &tenant_id, order_id).await {
        Ok(events) => Json(events).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
