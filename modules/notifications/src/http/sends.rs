//! HTTP endpoints for notification sends and delivery receipts.
//!
//!   POST /api/notifications/send         — send a notification
//!   GET  /api/notifications/{id}         — get send detail + receipts
//!   GET  /api/deliveries                 — query delivery receipts

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

use crate::event_bus::{create_notifications_envelope, enqueue_event};
use crate::sends::{models, repo};
use crate::template_store;

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct SendResponse {
    pub id: Uuid,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<i32>,
    pub channel: String,
    pub rendered_hash: Option<String>,
    pub receipt_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SendDetailResponse {
    pub id: Uuid,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<i32>,
    pub channel: String,
    pub rendered_hash: Option<String>,
    pub correlation_id: Option<String>,
    pub receipts: Vec<models::DeliveryReceipt>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ── Handlers ────────────────────────────────────────────────────────

#[utoipa::path(post, path = "/api/notifications/send", tag = "Sends",
    request_body = crate::sends::models::SendRequest,
    responses(
        (status = 201, description = "Notification sent", body = SendResponse),
        (status = 400, description = "Bad request", body = ApiError),
    ),
    security(("bearer" = [])))]
pub async fn send_notification(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(input): Json<models::SendRequest>,
) -> Result<(StatusCode, Json<SendResponse>), ApiError> {
    let tenant_id = require_tenant(&claims)?;

    let has_prerendered =
        input.rendered_subject.is_some() && input.rendered_body.is_some();
    let has_template = input.template_key.is_some();

    if !has_prerendered && !has_template {
        return Err(ApiError::bad_request(
            "Either template_key or both rendered_subject and rendered_body are required",
        ));
    }

    // Partial pre-rendered (one without the other) is invalid
    if input.rendered_subject.is_some() != input.rendered_body.is_some() {
        return Err(ApiError::bad_request(
            "Both rendered_subject and rendered_body must be provided together",
        ));
    }

    // Resolve rendered content: either from template or pre-rendered input.
    // When pre-rendered content is provided it takes precedence over template.
    let (rendered_subject, rendered_body, template_key, template_version) =
        if has_prerendered {
            let subj = input.rendered_subject.clone().unwrap();
            let body = input.rendered_body.clone().unwrap();
            (subj, body, input.template_key.clone(), None)
        } else {
            let tpl_key = input.template_key.as_ref().unwrap();
            let template =
                template_store::repo::get_latest(&pool, &tenant_id, tpl_key)
                    .await
                    .map_err(|e| ApiError::internal(e.to_string()))?
                    .ok_or_else(|| ApiError::bad_request("Template not found"))?;

            let (subj, body) =
                template_store::repo::render_template(&template, &input.payload_json)
                    .map_err(|e| ApiError::new(422, "validation_failed", e))?;

            (subj, body, Some(tpl_key.clone()), Some(template.version))
        };

    // Compute rendered hash for compliance proof
    let rendered_hash = compute_hash(&rendered_subject, &rendered_body);

    // Insert send record
    let send = repo::insert_send(
        &pool,
        &tenant_id,
        template_key.as_deref(),
        template_version,
        &input.channel,
        &input.recipients,
        &input.payload_json,
        input.correlation_id.as_deref(),
        input.causation_id.as_deref(),
        Some(&rendered_hash),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    // Create delivery receipts per recipient and emit events
    let mut receipt_count = 0;
    let mut any_succeeded = false;

    for recipient in &input.recipients {
        let receipt = repo::insert_receipt(
            &pool,
            &tenant_id,
            send.id,
            recipient,
            &input.channel,
            "succeeded",
            None,
            None,
            None,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        // Emit delivery.attempted + delivery.succeeded events
        let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

        let attempted_payload = serde_json::json!({
            "send_id": send.id,
            "receipt_id": receipt.id,
            "recipient": recipient,
            "channel": input.channel,
            "template_key": template_key,
            "template_version": template_version,
        });
        let attempted_envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.clone(),
            "notifications.events.delivery.attempted".to_string(),
            input.correlation_id.clone(),
            input.causation_id.clone(),
            "SIDE_EFFECT".to_string(),
            attempted_payload,
        );
        enqueue_event(
            &mut tx,
            "notifications.events.delivery.attempted",
            &attempted_envelope,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        let succeeded_payload = serde_json::json!({
            "send_id": send.id,
            "receipt_id": receipt.id,
            "recipient": recipient,
            "channel": input.channel,
            "template_key": template_key,
            "template_version": template_version,
            "rendered_hash": rendered_hash,
            "provider_id": receipt.provider_id,
        });
        let succeeded_envelope = create_notifications_envelope(
            Uuid::new_v4(),
            tenant_id.clone(),
            "notifications.events.delivery.succeeded".to_string(),
            input.correlation_id.clone(),
            input.causation_id.clone(),
            "SIDE_EFFECT".to_string(),
            succeeded_payload,
        );
        enqueue_event(
            &mut tx,
            "notifications.events.delivery.succeeded",
            &succeeded_envelope,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

        receipt_count += 1;
        any_succeeded = true;
    }

    // Update send status
    let final_status = if any_succeeded { "delivered" } else { "failed" };
    repo::update_send_status(&pool, send.id, final_status)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(SendResponse {
            id: send.id,
            status: final_status.to_string(),
            template_key,
            template_version,
            channel: input.channel,
            rendered_hash: Some(rendered_hash),
            receipt_count,
        }),
    ))
}

#[utoipa::path(get, path = "/api/notifications/{id}", tag = "Sends",
    params(("id" = Uuid, Path, description = "Send ID")),
    responses(
        (status = 200, description = "Send detail with receipts", body = SendDetailResponse),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_send_detail(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SendDetailResponse>, ApiError> {
    let tenant_id = require_tenant(&claims)?;

    let send = repo::get_send(&pool, &tenant_id, id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Notification send not found"))?;

    let receipts = repo::get_receipts_for_send(&pool, &tenant_id, id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(SendDetailResponse {
        id: send.id,
        status: send.status,
        template_key: send.template_key,
        template_version: send.template_version,
        channel: send.channel,
        rendered_hash: send.rendered_hash,
        correlation_id: send.correlation_id,
        receipts,
        created_at: send.created_at,
    }))
}

#[utoipa::path(get, path = "/api/deliveries", tag = "Sends",
    responses(
        (status = 200, description = "Delivery receipts", body = PaginatedResponse<crate::sends::models::DeliveryReceipt>),
    ),
    security(("bearer" = [])))]
pub async fn query_deliveries(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<models::DeliveryQuery>,
) -> Result<Json<PaginatedResponse<models::DeliveryReceipt>>, ApiError> {
    let tenant_id = require_tenant(&claims)?;
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let total = repo::count_receipts(
        &pool,
        &tenant_id,
        params.correlation_id.as_deref(),
        params.recipient.as_deref(),
        params.from,
        params.to,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let receipts = repo::query_receipts(
        &pool,
        &tenant_id,
        params.correlation_id.as_deref(),
        params.recipient.as_deref(),
        params.from,
        params.to,
        limit,
        offset,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let page = if limit > 0 { offset / limit + 1 } else { 1 };
    Ok(Json(PaginatedResponse::new(receipts, page, limit, total)))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn compute_hash(subject: &str, body: &str) -> String {
    let mut hasher = DefaultHasher::new();
    subject.hash(&mut hasher);
    body.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn require_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized("Missing or invalid authentication")),
    }
}

// ── Routers ─────────────────────────────────────────────────────────

pub fn sends_read_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/notifications/{id}", get(get_send_detail))
        .route("/api/deliveries", get(query_deliveries))
        .with_state(pool)
}

pub fn sends_mutate_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/notifications/send", post(send_notification))
        .with_state(pool)
}
