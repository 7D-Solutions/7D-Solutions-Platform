//! Operator-facing DLQ endpoints for dead-lettered notifications.
//!
//! Endpoints:
//!   GET  /api/dlq           — list dead-lettered items
//!   GET  /api/dlq/{id}      — fetch details + delivery attempts
//!   POST /api/dlq/{id}/replay  — replay (reset to pending)
//!   POST /api/dlq/{id}/abandon — mark as abandoned

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{create_notifications_envelope, enqueue_event};

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DlqItem {
    pub id: Uuid,
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: serde_json::Value,
    pub retry_count: i32,
    pub last_error: Option<String>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DlqListResponse {
    pub items: Vec<DlqItem>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct DeliveryAttemptDetail {
    pub id: Uuid,
    pub attempt_no: i32,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub rendered_subject: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DlqDetailResponse {
    pub item: DlqItem,
    pub delivery_attempts: Vec<DeliveryAttemptDetail>,
}

#[derive(Debug, Serialize)]
pub struct DlqActionResponse {
    pub id: Uuid,
    pub action: String,
    pub new_status: String,
}

#[derive(Debug, Serialize)]
pub struct DlqError {
    pub error: String,
    pub message: String,
}

// ── Query params ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DlqListParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub channel: Option<String>,
    pub template_key: Option<String>,
}

// ── Row types ───────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct DlqRow {
    id: Uuid,
    recipient_ref: String,
    channel: String,
    template_key: String,
    payload_json: serde_json::Value,
    retry_count: i32,
    last_error: Option<String>,
    dead_lettered_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AttemptRow {
    id: Uuid,
    attempt_no: i32,
    status: String,
    provider_message_id: Option<String>,
    error_class: Option<String>,
    error_message: Option<String>,
    rendered_subject: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct StatusOnly {
    status: String,
}

// ── Handlers ────────────────────────────────────────────────────────

async fn list_dlq(
    State(pool): State<PgPool>,
    Query(params): Query<DlqListParams>,
) -> Result<Json<DlqListResponse>, (StatusCode, Json<DlqError>)> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM scheduled_notifications WHERE status = 'dead_lettered'",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| internal_error(&e.to_string()))?;

    let mut query = String::from(
        "SELECT id, recipient_ref, channel, template_key, payload_json, \
         retry_count, last_error, dead_lettered_at, created_at \
         FROM scheduled_notifications WHERE status = 'dead_lettered'",
    );
    let mut bind_idx = 0u32;
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref ch) = params.channel {
        bind_idx += 1;
        query.push_str(&format!(" AND channel = ${bind_idx}"));
        binds.push(ch.clone());
    }
    if let Some(ref tk) = params.template_key {
        bind_idx += 1;
        query.push_str(&format!(" AND template_key = ${bind_idx}"));
        binds.push(tk.clone());
    }

    query.push_str(" ORDER BY dead_lettered_at DESC");
    bind_idx += 1;
    query.push_str(&format!(" LIMIT ${bind_idx}"));
    bind_idx += 1;
    query.push_str(&format!(" OFFSET ${bind_idx}"));

    let mut q = sqlx::query_as::<_, DlqRow>(&query);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(limit).bind(offset);

    let rows = q
        .fetch_all(&pool)
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    let items = rows
        .into_iter()
        .map(|r| DlqItem {
            id: r.id,
            recipient_ref: r.recipient_ref,
            channel: r.channel,
            template_key: r.template_key,
            payload_json: r.payload_json,
            retry_count: r.retry_count,
            last_error: r.last_error,
            dead_lettered_at: r.dead_lettered_at,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(DlqListResponse {
        items,
        total: count,
    }))
}

async fn get_dlq_item(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<DlqDetailResponse>, (StatusCode, Json<DlqError>)> {
    let row = sqlx::query_as::<_, DlqRow>(
        "SELECT id, recipient_ref, channel, template_key, payload_json, \
         retry_count, last_error, dead_lettered_at, created_at \
         FROM scheduled_notifications WHERE id = $1 AND status = 'dead_lettered'",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| internal_error(&e.to_string()))?
    .ok_or_else(|| not_found("DLQ item not found or not in dead_lettered status"))?;

    let attempts = sqlx::query_as::<_, AttemptRow>(
        "SELECT id, attempt_no, status, provider_message_id, error_class, \
         error_message, rendered_subject, created_at \
         FROM notification_delivery_attempts \
         WHERE notification_id = $1 ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(|e| internal_error(&e.to_string()))?;

    Ok(Json(DlqDetailResponse {
        item: DlqItem {
            id: row.id,
            recipient_ref: row.recipient_ref,
            channel: row.channel,
            template_key: row.template_key,
            payload_json: row.payload_json,
            retry_count: row.retry_count,
            last_error: row.last_error,
            dead_lettered_at: row.dead_lettered_at,
            created_at: row.created_at,
        },
        delivery_attempts: attempts
            .into_iter()
            .map(|a| DeliveryAttemptDetail {
                id: a.id,
                attempt_no: a.attempt_no,
                status: a.status,
                provider_message_id: a.provider_message_id,
                error_class: a.error_class,
                error_message: a.error_message,
                rendered_subject: a.rendered_subject,
                created_at: a.created_at,
            })
            .collect(),
    }))
}

async fn replay_dlq_item(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<DlqActionResponse>, (StatusCode, Json<DlqError>)> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    // Guard: only replay if currently dead_lettered (SELECT FOR UPDATE prevents races)
    let current = sqlx::query_as::<_, StatusOnly>(
        "SELECT status FROM scheduled_notifications WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal_error(&e.to_string()))?
    .ok_or_else(|| not_found("Notification not found"))?;

    if current.status != "dead_lettered" {
        // Idempotent: if already replayed (pending/attempting/sent) or abandoned, return current state
        tx.commit()
            .await
            .map_err(|e| internal_error(&e.to_string()))?;
        return Ok(Json(DlqActionResponse {
            id,
            action: "replay".to_string(),
            new_status: current.status,
        }));
    }

    // Mutation: reset to pending for re-dispatch, bump replay_generation
    // for fresh idempotency keys
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'pending', \
             deliver_at = NOW(), \
             retry_count = 0, \
             replay_generation = replay_generation + 1, \
             last_error = NULL, \
             dead_lettered_at = NULL, \
             failed_at = NULL \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| internal_error(&e.to_string()))?;

    // Outbox: emit dlq.replayed event
    let tenant_id = extract_tenant_id(&pool, id)
        .await
        .unwrap_or_else(|| "unknown".to_string());
    let envelope = create_notifications_envelope(
        Uuid::new_v4(),
        tenant_id,
        "notifications.dlq.replayed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        serde_json::json!({
            "notification_id": id,
            "action": "replay",
            "previous_status": "dead_lettered",
            "new_status": "pending",
        }),
    );
    enqueue_event(&mut tx, "notifications.events.dlq.replayed", &envelope)
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    tracing::info!(notification_id = %id, "DLQ item replayed — reset to pending");

    Ok(Json(DlqActionResponse {
        id,
        action: "replay".to_string(),
        new_status: "pending".to_string(),
    }))
}

async fn abandon_dlq_item(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<DlqActionResponse>, (StatusCode, Json<DlqError>)> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    // Guard: only abandon if currently dead_lettered
    let current = sqlx::query_as::<_, StatusOnly>(
        "SELECT status FROM scheduled_notifications WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal_error(&e.to_string()))?
    .ok_or_else(|| not_found("Notification not found"))?;

    if current.status != "dead_lettered" {
        // Idempotent: if already abandoned or in another state, return current
        tx.commit()
            .await
            .map_err(|e| internal_error(&e.to_string()))?;
        return Ok(Json(DlqActionResponse {
            id,
            action: "abandon".to_string(),
            new_status: current.status,
        }));
    }

    // Mutation: mark as abandoned
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'abandoned', \
             abandoned_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| internal_error(&e.to_string()))?;

    // Outbox: emit dlq.abandoned event
    let tenant_id = extract_tenant_id(&pool, id)
        .await
        .unwrap_or_else(|| "unknown".to_string());
    let envelope = create_notifications_envelope(
        Uuid::new_v4(),
        tenant_id,
        "notifications.dlq.abandoned".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        serde_json::json!({
            "notification_id": id,
            "action": "abandon",
            "previous_status": "dead_lettered",
            "new_status": "abandoned",
        }),
    );
    enqueue_event(&mut tx, "notifications.events.dlq.abandoned", &envelope)
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    tracing::info!(notification_id = %id, "DLQ item abandoned by operator");

    Ok(Json(DlqActionResponse {
        id,
        action: "abandon".to_string(),
        new_status: "abandoned".to_string(),
    }))
}

// ── Helpers ─────────────────────────────────────────────────────────

async fn extract_tenant_id(pool: &PgPool, notification_id: Uuid) -> Option<String> {
    #[derive(sqlx::FromRow)]
    struct RecipientRow {
        recipient_ref: String,
    }

    let row = sqlx::query_as::<_, RecipientRow>(
        "SELECT recipient_ref FROM scheduled_notifications WHERE id = $1",
    )
    .bind(notification_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    // Convention: recipient_ref is "tenant_id:user_ref" or just an email
    row.recipient_ref
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn internal_error(msg: &str) -> (StatusCode, Json<DlqError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(DlqError {
            error: "internal_error".to_string(),
            message: msg.to_string(),
        }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<DlqError>) {
    (
        StatusCode::NOT_FOUND,
        Json(DlqError {
            error: "not_found".to_string(),
            message: msg.to_string(),
        }),
    )
}

// ── Router ──────────────────────────────────────────────────────────

/// Build separate read and mutate routers so main.rs can apply different
/// permission layers (NOTIFICATIONS_READ vs NOTIFICATIONS_MUTATE).
pub fn dlq_read_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/dlq", get(list_dlq))
        .route("/api/dlq/{id}", get(get_dlq_item))
        .with_state(pool)
}

pub fn dlq_mutate_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/dlq/{id}/replay", post(replay_dlq_item))
        .route("/api/dlq/{id}/abandon", post(abandon_dlq_item))
        .with_state(pool)
}
