//! HTTP handlers for ship events.

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

pub async fn create_ship_event(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CreateShipEventRequest>,
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

    // Row-lock order and check quantity bounds
    let (order, sum_shipped, _sum_received) =
        match repo::lock_order_for_quantity_check(&mut tx, &tenant_id, order_id).await {
            Ok(v) => v,
            Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
        };

    // Validate state allows ship event
    let new_status = match state_machine::transition_on_ship_event(&order.status) {
        Ok(s) => s,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    // Validate quantity bound
    let new_total = sum_shipped + req.quantity_shipped as i64;
    if new_total > order.quantity_sent as i64 {
        return with_request_id(
            ApiError::new(422, "quantity_exceeded",
                format!("Ship quantity {} would exceed quantity_sent {} (already shipped: {})",
                    req.quantity_shipped, order.quantity_sent, sum_shipped)),
            &tracing_ctx,
        ).into_response();
    }

    let ship_event = match repo::create_ship_event_tx(&mut tx, &tenant_id, order_id, &req).await {
        Ok(e) => e,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    // Advance order status
    if let Err(e) = repo::set_order_status(&mut tx, &tenant_id, order_id, new_status.as_str()).await {
        return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
    }

    // Emit shipment_requested event
    let req_event_id = Uuid::new_v4();
    let req_env = events::build_shipment_requested_envelope(
        req_event_id, tenant_id.clone(), correlation_id.clone(), None,
        ShipmentRequestedPayload {
            op_order_id: order_id,
            ship_event_id: ship_event.id,
            tenant_id: tenant_id.clone(),
            vendor_id: order.vendor_id,
            quantity_shipped: req.quantity_shipped,
            lot_number: req.lot_number.clone(),
            part_number: order.part_number.clone(),
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, req_event_id,
        events::EVENT_SHIPMENT_REQUESTED, "op_order", &order_id.to_string(),
        &req_env, &correlation_id, None,
    ).await;

    // Emit shipped event
    let shipped_event_id = Uuid::new_v4();
    let shipped_env = events::build_shipped_envelope(
        shipped_event_id, tenant_id.clone(), correlation_id.clone(), None,
        ShippedPayload {
            op_order_id: order_id,
            ship_event_id: ship_event.id,
            tenant_id: tenant_id.clone(),
            quantity_shipped: req.quantity_shipped,
            ship_date: req.ship_date,
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, shipped_event_id,
        events::EVENT_SHIPPED, "op_order", &order_id.to_string(),
        &shipped_env, &correlation_id, None,
    ).await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.ship_events_recorded.inc();
    (StatusCode::CREATED, Json(ship_event)).into_response()
}

pub async fn list_ship_events(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_ship_events(&state.pool, &tenant_id, order_id).await {
        Ok(events) => Json(events).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
