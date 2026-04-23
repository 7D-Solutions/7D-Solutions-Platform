//! HTTP handlers for complaint lifecycle — 10 endpoints per spec §4.1.

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

use crate::domain::{
    models::{
        AssignComplaintRequest, CancelComplaintRequest, CloseComplaintRequest,
        CreateComplaintRequest, ListComplaintsQuery, RespondComplaintRequest,
        StartInvestigationRequest, TriageComplaintRequest, UpdateComplaintRequest,
    },
    repo,
};
use crate::events::produced::{
    self as ev, ComplaintAssignedPayload, ComplaintClosedPayload, ComplaintReceivedPayload,
    ComplaintStatusChangedPayload, ComplaintTriagedPayload,
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

// ── Create ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints", tag = "Complaints",
    request_body = CreateComplaintRequest,
    responses(
        (status = 201, description = "Complaint created", body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateComplaintRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response()
        }
    };
    let number = match repo::next_complaint_number(&mut tx, &tenant_id).await {
        Ok(n) => n,
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };
    let complaint = match repo::create_complaint(&mut tx, &tenant_id, &req, &number).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let payload = ComplaintReceivedPayload {
        complaint_id: complaint.id,
        complaint_number: complaint.complaint_number.clone(),
        tenant_id: tenant_id.clone(),
        party_id: complaint.party_id,
        source: complaint.source.clone(),
        severity: complaint.severity.clone(),
        category_code: complaint.category_code.clone(),
        source_entity_type: complaint.source_entity_type.clone(),
        source_entity_id: complaint.source_entity_id,
        received_at: complaint.received_at,
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_RECEIVED,
        complaint.id,
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
    (StatusCode::CREATED, Json(complaint)).into_response()
}

// ── Get ───────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/complaints/{id}", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    responses(
        (status = 200, body = crate::domain::models::ComplaintDetail),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::get_complaint_detail(&state.pool, &tenant_id, id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Complaint {} not found", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── List ──────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/complaints", tag = "Complaints",
    responses((status = 200, body = PaginatedResponse<crate::domain::models::Complaint>)),
    security(("bearer" = [])),
)]
pub async fn list_complaints(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListComplaintsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::list_complaints(&state.pool, &tenant_id, &query).await {
        Ok(items) => {
            let total = items.len() as i64;
            Json(PaginatedResponse::new(items, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── Update ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    put, path = "/api/customer-complaints/complaints/{id}", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = UpdateComplaintRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 404, body = ApiError),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateComplaintRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::update_complaint(&state.pool, &tenant_id, id, &req).await {
        Ok(c) => Json(c).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── Triage ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/triage", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = TriageComplaintRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn triage_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<TriageComplaintRequest>,
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
    let c = match repo::triage_complaint(&mut tx, &tenant_id, id, &req).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let triaged_event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, triaged_event_id);
    let triaged_payload = ComplaintTriagedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        assigned_to: req.assigned_to.clone(),
        category_code: req.category_code.clone(),
        severity: req.severity.as_str().to_string(),
        triaged_at: c.assigned_at.unwrap_or_else(chrono::Utc::now),
    };
    let status_event_id = Uuid::new_v4();
    let status_payload = ComplaintStatusChangedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        from_status: "intake".to_string(),
        to_status: "triaged".to_string(),
        transitioned_by: req.triaged_by.clone(),
        transitioned_at: c.updated_at,
    };
    let enqueue = outbox::enqueue_event_tx(
        &mut tx,
        triaged_event_id,
        ev::EVENT_COMPLAINT_TRIAGED,
        c.id,
        &tenant_id,
        Some(&corr),
        None,
        &triaged_payload,
    )
    .await
    .and(
        outbox::enqueue_event_tx(
            &mut tx,
            status_event_id,
            ev::EVENT_COMPLAINT_STATUS_CHANGED,
            c.id,
            &tenant_id,
            Some(&corr),
            Some(&triaged_event_id.to_string()),
            &status_payload,
        )
        .await,
    );
    if let Err(e) = enqueue {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    Json(c).into_response()
}

// ── Start Investigation ───────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/start-investigation", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = StartInvestigationRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn start_investigation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<StartInvestigationRequest>,
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
    let c = match repo::start_investigation(&mut tx, &tenant_id, id, &req).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let status_payload = ComplaintStatusChangedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        from_status: "triaged".to_string(),
        to_status: "investigating".to_string(),
        transitioned_by: req.started_by.clone(),
        transitioned_at: c.updated_at,
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_STATUS_CHANGED,
        c.id,
        &tenant_id,
        Some(&corr),
        None,
        &status_payload,
    )
    .await
    {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    Json(c).into_response()
}

// ── Respond ───────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/respond", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = RespondComplaintRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn respond_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RespondComplaintRequest>,
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
    let c = match repo::respond_complaint(&mut tx, &tenant_id, id, &req).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let status_payload = ComplaintStatusChangedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        from_status: "investigating".to_string(),
        to_status: "responded".to_string(),
        transitioned_by: req.responded_by.clone(),
        transitioned_at: c.updated_at,
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_STATUS_CHANGED,
        c.id,
        &tenant_id,
        Some(&corr),
        None,
        &status_payload,
    )
    .await
    {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    Json(c).into_response()
}

// ── Close ─────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/close", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = CloseComplaintRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn close_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CloseComplaintRequest>,
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
    let c = match repo::close_complaint(&mut tx, &tenant_id, id, &req).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let closed_event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, closed_event_id);
    let closed_payload = ComplaintClosedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        outcome: req.outcome.as_str().to_string(),
        closed_at: c.closed_at.unwrap_or_else(chrono::Utc::now),
    };
    let status_event_id = Uuid::new_v4();
    let status_payload = ComplaintStatusChangedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        from_status: "responded".to_string(),
        to_status: "closed".to_string(),
        transitioned_by: req.closed_by.clone(),
        transitioned_at: c.updated_at,
    };
    let enqueue = outbox::enqueue_event_tx(
        &mut tx,
        closed_event_id,
        ev::EVENT_COMPLAINT_CLOSED,
        c.id,
        &tenant_id,
        Some(&corr),
        None,
        &closed_payload,
    )
    .await
    .and(
        outbox::enqueue_event_tx(
            &mut tx,
            status_event_id,
            ev::EVENT_COMPLAINT_STATUS_CHANGED,
            c.id,
            &tenant_id,
            Some(&corr),
            Some(&closed_event_id.to_string()),
            &status_payload,
        )
        .await,
    );
    if let Err(e) = enqueue {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    Json(c).into_response()
}

// ── Cancel ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/cancel", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = CancelComplaintRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn cancel_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CancelComplaintRequest>,
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
    let c = match repo::cancel_complaint(&mut tx, &tenant_id, id, &req).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let status_payload = ComplaintStatusChangedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        from_status: "unknown".to_string(),
        to_status: "cancelled".to_string(),
        transitioned_by: req.cancelled_by.clone(),
        transitioned_at: c.updated_at,
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_STATUS_CHANGED,
        c.id,
        &tenant_id,
        Some(&corr),
        None,
        &status_payload,
    )
    .await
    {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    Json(c).into_response()
}

// ── Assign ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/customer-complaints/complaints/{id}/assign", tag = "Complaints",
    params(("id" = Uuid, Path, description = "Complaint ID")),
    request_body = AssignComplaintRequest,
    responses(
        (status = 200, body = crate::domain::models::Complaint),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn assign_complaint(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignComplaintRequest>,
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
    let c = match repo::assign_complaint(&mut tx, &tenant_id, id, &req).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.rollback().await;
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
    };
    let event_id = Uuid::new_v4();
    let corr = correlation_id(&tracing_ctx, event_id);
    let assigned_payload = ComplaintAssignedPayload {
        complaint_id: c.id,
        tenant_id: tenant_id.clone(),
        from_user: None,
        to_user: req.assigned_to.clone(),
        assigned_by: req.assigned_by.clone(),
        assigned_at: c.assigned_at.unwrap_or_else(chrono::Utc::now),
    };
    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        ev::EVENT_COMPLAINT_ASSIGNED,
        c.id,
        &tenant_id,
        Some(&corr),
        None,
        &assigned_payload,
    )
    .await
    {
        let _ = tx.rollback().await;
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    if let Err(e) = tx.commit().await {
        return with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response();
    }
    Json(c).into_response()
}
