//! HTTP handlers for vendor reviews (append-only).

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

pub async fn create_review(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CreateReviewRequest>,
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

    let (order, _, _) = match repo::lock_order_for_quantity_check(&mut tx, &tenant_id, order_id).await {
        Ok(v) => v,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    // review-follows-return: must have at least one return event
    let return_count = match repo::count_return_events(&mut tx, order_id).await {
        Ok(c) => c,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };
    if return_count == 0 {
        return with_request_id(
            ApiError::new(422, "review_before_return",
                "Cannot record a review before any return event exists"),
            &tracing_ctx,
        ).into_response();
    }

    // Verify the referenced return_event_id belongs to this order
    let return_exists = match repo::return_event_exists(&mut tx, &tenant_id, order_id, req.return_event_id).await {
        Ok(e) => e,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };
    if !return_exists {
        return with_request_id(
            ApiError::not_found(format!("Return event {} not found for this order", req.return_event_id)),
            &tracing_ctx,
        ).into_response();
    }

    // State transition into review_in_progress (or stay if already there)
    let _ = state_machine::transition_on_review_created(&order.status).map_err(|e| {
        // Only fail if state doesn't allow review creation at all
        e
    });

    let rework = req.rework.unwrap_or(false);
    let new_status = match state_machine::transition_on_review_outcome(&order.status, req.outcome, rework) {
        Ok(s) => s,
        Err(_) => {
            // Order may be in 'returned', first move to review_in_progress
            match state_machine::transition_on_review_created(&order.status) {
                Ok(review_status) => {
                    match state_machine::transition_on_review_outcome(review_status.as_str(), req.outcome, rework) {
                        Ok(s) => s,
                        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
                    }
                }
                Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
            }
        }
    };

    let review = match repo::create_review_tx(&mut tx, &tenant_id, order_id, &req).await {
        Ok(r) => r,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    if let Err(e) = repo::set_order_status(&mut tx, &tenant_id, order_id, new_status.as_str()).await {
        return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
    }

    let event_id = Uuid::new_v4();
    let env = events::build_review_completed_envelope(
        event_id, tenant_id.clone(), correlation_id.clone(), None,
        ReviewCompletedPayload {
            op_order_id: order_id,
            review_id: review.id,
            tenant_id: tenant_id.clone(),
            outcome: req.outcome.as_str().to_string(),
            reviewed_at: req.reviewed_at,
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx, &tenant_id, event_id,
        events::EVENT_REVIEW_COMPLETED, "op_order", &order_id.to_string(),
        &env, &correlation_id, None,
    ).await;

    if new_status == OpOrderStatus::Closed {
        let close_event_id = Uuid::new_v4();
        let close_env = events::build_order_closed_envelope(
            close_event_id, tenant_id.clone(), correlation_id.clone(), None,
            OrderClosedPayload {
                op_order_id: order_id,
                tenant_id: tenant_id.clone(),
                closed_at: req.reviewed_at,
                final_accepted_qty: order.quantity_sent,
            },
        );
        let _ = repo::enqueue_outbox(
            &mut tx, &tenant_id, close_event_id,
            events::EVENT_ORDER_CLOSED, "op_order", &order_id.to_string(),
            &close_env, &correlation_id, Some(&event_id.to_string()),
        ).await;
        state.metrics.orders_closed.inc();
    }

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    state.metrics.reviews_recorded.inc();
    (StatusCode::CREATED, Json(review)).into_response()
}

pub async fn list_reviews(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_reviews(&state.pool, &tenant_id, order_id).await {
        Ok(reviews) => Json(reviews).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// Reviews are append-only — PUT/DELETE return 405.
pub async fn method_not_allowed() -> impl IntoResponse {
    (StatusCode::METHOD_NOT_ALLOWED, Json(serde_json::json!({
        "error": "method_not_allowed",
        "message": "Vendor reviews are append-only. Create a new review record to add a correction."
    })))
}
