//! HTTP handlers for re-identification records.

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

use crate::domain::{models::*, repo};
use crate::events::{self, *};
use crate::http::tenant::{correlation_from_headers, with_request_id};
use crate::AppState;
use platform_sdk::extract_tenant;

pub async fn create_re_identification(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<CreateReIdentificationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    let mut tx = match state.pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx)
                .into_response()
        }
    };

    // Re-identification requires a return event
    let return_count = match repo::count_return_events(&mut tx, order_id).await {
        Ok(c) => c,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };
    if return_count == 0 {
        return with_request_id(
            ApiError::new(
                422,
                "no_return_event",
                "Cannot record re-identification before any return event exists",
            ),
            &tracing_ctx,
        )
        .into_response();
    }

    let return_exists =
        match repo::return_event_exists(&mut tx, &tenant_id, order_id, req.return_event_id).await {
            Ok(e) => e,
            Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
        };
    if !return_exists {
        return with_request_id(
            ApiError::not_found(format!(
                "Return event {} not found for this order",
                req.return_event_id
            )),
            &tracing_ctx,
        )
        .into_response();
    }

    let reid = match repo::create_re_identification_tx(&mut tx, &tenant_id, order_id, &req).await {
        Ok(r) => r,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

    let event_id = Uuid::new_v4();
    let env = events::build_re_identification_envelope(
        event_id,
        tenant_id.clone(),
        correlation_id.clone(),
        None,
        ReIdentificationRecordedPayload {
            op_order_id: order_id,
            tenant_id: tenant_id.clone(),
            old_part_number: req.old_part_number.clone(),
            new_part_number: req.new_part_number.clone(),
            performed_at: req.performed_at,
        },
    );
    let _ = repo::enqueue_outbox(
        &mut tx,
        &tenant_id,
        event_id,
        events::EVENT_RE_IDENTIFICATION_RECORDED,
        "op_order",
        &order_id.to_string(),
        &env,
        &correlation_id,
        None,
    )
    .await;

    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::from(OpError::Database(e)), &tracing_ctx).into_response();
    }

    (StatusCode::CREATED, Json(reid)).into_response()
}

pub async fn list_re_identifications(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(order_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match repo::list_re_identifications(&state.pool, &tenant_id, order_id).await {
        Ok(reids) => Json(reids).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
