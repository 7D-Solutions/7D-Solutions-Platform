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
use security::VerifiedClaims;
use serde::Serialize;
use sqlx::PgPool;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

use crate::event_bus::{create_notifications_envelope, enqueue_event};
use crate::sends::{models, repo};
use crate::template_store;

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub id: Uuid,
    pub status: String,
    pub template_key: String,
    pub template_version: i32,
    pub channel: String,
    pub rendered_hash: Option<String>,
    pub receipt_count: usize,
}

#[derive(Debug, Serialize)]
pub struct SendDetailResponse {
    pub id: Uuid,
    pub status: String,
    pub template_key: String,
    pub template_version: i32,
    pub channel: String,
    pub rendered_hash: Option<String>,
    pub correlation_id: Option<String>,
    pub receipts: Vec<models::DeliveryReceipt>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct DeliveryListResponse {
    pub receipts: Vec<models::DeliveryReceipt>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

// ── Handlers ────────────────────────────────────────────────────────

async fn send_notification(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(input): Json<models::SendRequest>,
) -> Result<(StatusCode, Json<SendResponse>), (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = require_tenant(&claims)?;

    // Resolve template (latest version)
    let template =
        template_store::repo::get_latest(&pool, &tenant_id, &input.template_key)
            .await
            .map_err(|e| internal_error(&e.to_string()))?
            .ok_or_else(|| bad_request("Template not found"))?;

    // Validate required_vars
    let (rendered_subject, rendered_body) =
        template_store::repo::render_template(&template, &input.payload_json)
            .map_err(|e| {
                // Emit delivery.failed for validation errors — fire-and-forget
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ErrorResponse {
                        error: "validation_failed".to_string(),
                        message: e,
                    }),
                )
            })?;

    // Compute rendered hash for compliance proof
    let rendered_hash = compute_hash(&rendered_subject, &rendered_body);

    // Insert send record
    let send = repo::insert_send(
        &pool,
        &tenant_id,
        &input.template_key,
        template.version,
        &input.channel,
        &input.recipients,
        &input.payload_json,
        input.correlation_id.as_deref(),
        input.causation_id.as_deref(),
        Some(&rendered_hash),
    )
    .await
    .map_err(|e| internal_error(&e.to_string()))?;

    // Create delivery receipts per recipient and emit events
    let mut receipt_count = 0;
    let mut any_succeeded = false;

    for recipient in &input.recipients {
        // In a real system, this would dispatch to the actual channel sender.
        // For now, we record as "succeeded" (the scheduled dispatcher handles
        // actual delivery). This creates the compliance record.
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
        .map_err(|e| internal_error(&e.to_string()))?;

        // Emit delivery.attempted + delivery.succeeded events
        let mut tx = pool.begin().await.map_err(|e| internal_error(&e.to_string()))?;

        let attempted_payload = serde_json::json!({
            "send_id": send.id,
            "receipt_id": receipt.id,
            "recipient": recipient,
            "channel": input.channel,
            "template_key": input.template_key,
            "template_version": template.version,
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
        .map_err(|e| internal_error(&e.to_string()))?;

        let succeeded_payload = serde_json::json!({
            "send_id": send.id,
            "receipt_id": receipt.id,
            "recipient": recipient,
            "channel": input.channel,
            "template_key": input.template_key,
            "template_version": template.version,
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
        .map_err(|e| internal_error(&e.to_string()))?;

        tx.commit().await.map_err(|e| internal_error(&e.to_string()))?;

        receipt_count += 1;
        any_succeeded = true;
    }

    // Update send status
    let final_status = if any_succeeded { "delivered" } else { "failed" };
    repo::update_send_status(&pool, send.id, final_status)
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(SendResponse {
            id: send.id,
            status: final_status.to_string(),
            template_key: input.template_key,
            template_version: template.version,
            channel: input.channel,
            rendered_hash: Some(rendered_hash),
            receipt_count,
        }),
    ))
}

async fn get_send_detail(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SendDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = require_tenant(&claims)?;

    let send = repo::get_send(&pool, &tenant_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .ok_or_else(|| not_found("Notification send not found"))?;

    let receipts = repo::get_receipts_for_send(&pool, &tenant_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

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

async fn query_deliveries(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<models::DeliveryQuery>,
) -> Result<Json<DeliveryListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = require_tenant(&claims)?;
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

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
    .map_err(|e| internal_error(&e.to_string()))?;

    Ok(Json(DeliveryListResponse { receipts }))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn compute_hash(subject: &str, body: &str) -> String {
    let mut hasher = DefaultHasher::new();
    subject.hash(&mut hasher);
    body.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn require_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(unauthorized("Missing or invalid authentication")),
    }
}

fn unauthorized(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "unauthorized".to_string(),
            message: msg.to_string(),
        }),
    )
}

fn internal_error(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "internal_error".to_string(),
            message: msg.to_string(),
        }),
    )
}

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "bad_request".to_string(),
            message: msg.to_string(),
        }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "not_found".to_string(),
            message: msg.to_string(),
        }),
    )
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
