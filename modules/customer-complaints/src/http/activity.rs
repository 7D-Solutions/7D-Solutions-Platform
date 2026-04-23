//! HTTP handlers for activity log and resolution — per spec §4.2.

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

use crate::domain::{
    models::{ActivityType, CreateActivityLogRequest, CreateResolutionRequest},
    repo,
};
use crate::events::produced::{
    self as ev, ComplaintCustomerCommunicatedPayload, ComplaintResolvedPayload,
};
use crate::http::tenant::with_request_id;
use crate::outbox;
use crate::AppState;
use platform_sdk::extract_tenant;

fn correlation_id(tracing_ctx: &Option<Extension<TracingContext>>, event_id: Uuid) -> String {
    tracing_ctx
        .as_ref()
        .and_then(|ext| ext.trace_id.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| event_id.to_string())
}

// ── Add note ──────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/notes", tag = "Activity",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = CreateActivityLogRequest,
    responses(
        (status = 201, body = crate::domain::models::ComplaintActivityLog),
        (status = 404, body = ApiError),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_note(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<CreateActivityLogRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.activity_type = ActivityType::Note;
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response()
        }
    };
    match repo::add_activity_log_entry(&mut tx, &tenant_id, id, &req).await {
        Ok(entry) => {
            if let Err(e) = tx.commit().await {
                return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx)
                    .into_response();
            }
            (StatusCode::CREATED, Json(entry)).into_response()
        }
        Err(e) => {
            let _ = tx.rollback().await;
            with_request_id(ApiError::from(e), &tracing_ctx).into_response()
        }
    }
}

// ── Add customer communication ────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/customer-communication", tag = "Activity",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = CreateActivityLogRequest,
    responses(
        (status = 201, body = crate::domain::models::ComplaintActivityLog),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_customer_communication(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<CreateActivityLogRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.activity_type = ActivityType::CustomerCommunication;
    let recorded_by = req.recorded_by.clone();
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response()
        }
    };
    let entry = match repo::add_activity_log_entry(&mut tx, &tenant_id, id, &req).await {
        Ok(e) => e,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let payload = ComplaintCustomerCommunicatedPayload {
        complaint_id: id,
        tenant_id: tenant_id.clone(),
        recorded_by,
        recorded_at: entry.recorded_at,
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_CUSTOMER_COMMUNICATED,
        id,
        &tenant_id,
        Some(&corr),
        None,
        &payload,
    )
    .await
    {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    (StatusCode::CREATED, Json(entry)).into_response()
}

// ── List activity log ─────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/complaints/{id}/activity-log", tag = "Activity",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    responses((status = 200, body = Vec<crate::domain::models::ComplaintActivityLog>)),
    security(("bearer" = [])),
)]
pub async fn list_activity_log(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::list_activity_log(&state.pool, &tenant_id, id).await {
        Ok(log) => Json(log).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── Create resolution ─────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/resolution", tag = "Activity",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = CreateResolutionRequest,
    responses(
        (status = 201, body = crate::domain::models::ComplaintResolution),
        (status = 409, body = ApiError),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_resolution(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateResolutionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response()
        }
    };
    let resolved_by = req.resolved_by.clone();
    let customer_acceptance = req.customer_acceptance.as_str().to_string();
    let res = match repo::create_resolution(&mut tx, &tenant_id, id, &req).await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let payload = ComplaintResolvedPayload {
        complaint_id: id,
        tenant_id: tenant_id.clone(),
        customer_acceptance,
        resolved_by,
        resolved_at: res.resolved_at,
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_RESOLVED,
        id,
        &tenant_id,
        Some(&corr),
        None,
        &payload,
    )
    .await
    {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    (StatusCode::CREATED, Json(res)).into_response()
}

// ── Get resolution ────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/complaints/{id}/resolution", tag = "Activity",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    responses(
        (status = 200, body = crate::domain::models::ComplaintResolution),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_resolution(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::get_resolution(&state.pool, &tenant_id, id).await {
        Ok(Some(res)) => Json(res).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("No resolution found for complaint {}", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
