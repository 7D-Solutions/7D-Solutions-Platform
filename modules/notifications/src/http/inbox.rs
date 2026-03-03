//! Per-user in-app inbox API endpoints.
//!
//! Endpoints:
//!   GET  /api/inbox           — list inbox messages (pagination + filters)
//!   GET  /api/inbox/{id}      — fetch single message
//!   POST /api/inbox/{id}/read     — mark as read
//!   POST /api/inbox/{id}/unread   — mark as unread
//!   POST /api/inbox/{id}/dismiss  — dismiss
//!   POST /api/inbox/{id}/undismiss — undismiss

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::inbox;

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct InboxItem {
    pub id: Uuid,
    pub notification_id: Uuid,
    pub title: String,
    pub body: Option<String>,
    pub category: Option<String>,
    pub is_read: bool,
    pub is_dismissed: bool,
    pub read_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct InboxListResponse {
    pub items: Vec<InboxItem>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct InboxActionResponse {
    pub id: Uuid,
    pub action: String,
    pub is_read: bool,
    pub is_dismissed: bool,
}

#[derive(Debug, Serialize)]
pub struct InboxError {
    pub error: String,
    pub message: String,
}

// ── Query params ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InboxListQuery {
    pub user_id: String,
    pub unread_only: Option<bool>,
    pub include_dismissed: Option<bool>,
    pub category: Option<String>,
    pub page_size: Option<i64>,
    pub offset: Option<i64>,
}

// ── Handlers ────────────────────────────────────────────────────────

async fn list_inbox(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<InboxListQuery>,
) -> Result<Json<InboxListResponse>, (StatusCode, Json<InboxError>)> {
    let tenant_id = require_tenant(&claims)?;

    let list_params = inbox::InboxListParams {
        tenant_id,
        user_id: params.user_id,
        unread_only: params.unread_only.unwrap_or(false),
        include_dismissed: params.include_dismissed.unwrap_or(false),
        category: params.category,
        limit: params.page_size.unwrap_or(25).min(200),
        offset: params.offset.unwrap_or(0),
    };

    let (messages, total) = inbox::list_messages(&pool, &list_params)
        .await
        .map_err(|e| internal_error(&e.to_string()))?;

    let items = messages.into_iter().map(to_inbox_item).collect();
    Ok(Json(InboxListResponse { items, total }))
}

async fn get_inbox_message(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<InboxItem>, (StatusCode, Json<InboxError>)> {
    let tenant_id = require_tenant(&claims)?;

    let msg = inbox::get_message(&pool, &tenant_id, &params.user_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .ok_or_else(|| not_found("Inbox message not found"))?;

    Ok(Json(to_inbox_item(msg)))
}

#[derive(Debug, Deserialize)]
pub struct UserIdQuery {
    pub user_id: String,
}

async fn read_message(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<InboxActionResponse>, (StatusCode, Json<InboxError>)> {
    let tenant_id = require_tenant(&claims)?;

    let msg = inbox::mark_read(&pool, &tenant_id, &params.user_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .ok_or_else(|| not_found("Inbox message not found"))?;

    Ok(Json(InboxActionResponse {
        id: msg.id,
        action: "read".to_string(),
        is_read: msg.is_read,
        is_dismissed: msg.is_dismissed,
    }))
}

async fn unread_message(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<InboxActionResponse>, (StatusCode, Json<InboxError>)> {
    let tenant_id = require_tenant(&claims)?;

    let msg = inbox::mark_unread(&pool, &tenant_id, &params.user_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .ok_or_else(|| not_found("Inbox message not found"))?;

    Ok(Json(InboxActionResponse {
        id: msg.id,
        action: "unread".to_string(),
        is_read: msg.is_read,
        is_dismissed: msg.is_dismissed,
    }))
}

async fn dismiss_inbox_message(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<InboxActionResponse>, (StatusCode, Json<InboxError>)> {
    let tenant_id = require_tenant(&claims)?;

    let msg = inbox::dismiss_message(&pool, &tenant_id, &params.user_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .ok_or_else(|| not_found("Inbox message not found"))?;

    Ok(Json(InboxActionResponse {
        id: msg.id,
        action: "dismiss".to_string(),
        is_read: msg.is_read,
        is_dismissed: msg.is_dismissed,
    }))
}

async fn undismiss_inbox_message(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<InboxActionResponse>, (StatusCode, Json<InboxError>)> {
    let tenant_id = require_tenant(&claims)?;

    let msg = inbox::undismiss_message(&pool, &tenant_id, &params.user_id, id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .ok_or_else(|| not_found("Inbox message not found"))?;

    Ok(Json(InboxActionResponse {
        id: msg.id,
        action: "undismiss".to_string(),
        is_read: msg.is_read,
        is_dismissed: msg.is_dismissed,
    }))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn to_inbox_item(m: inbox::InboxMessage) -> InboxItem {
    InboxItem {
        id: m.id,
        notification_id: m.notification_id,
        title: m.title,
        body: m.body,
        category: m.category,
        is_read: m.is_read,
        is_dismissed: m.is_dismissed,
        read_at: m.read_at,
        dismissed_at: m.dismissed_at,
        created_at: m.created_at,
    }
}

fn require_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<InboxError>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(unauthorized("Missing or invalid authentication")),
    }
}

fn unauthorized(msg: &str) -> (StatusCode, Json<InboxError>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(InboxError {
            error: "unauthorized".to_string(),
            message: msg.to_string(),
        }),
    )
}

fn internal_error(msg: &str) -> (StatusCode, Json<InboxError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(InboxError {
            error: "internal_error".to_string(),
            message: msg.to_string(),
        }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<InboxError>) {
    (
        StatusCode::NOT_FOUND,
        Json(InboxError {
            error: "not_found".to_string(),
            message: msg.to_string(),
        }),
    )
}

// ── Routers ─────────────────────────────────────────────────────────

pub fn inbox_read_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/inbox", get(list_inbox))
        .route("/api/inbox/{id}", get(get_inbox_message))
        .with_state(pool)
}

pub fn inbox_mutate_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/inbox/{id}/read", post(read_message))
        .route("/api/inbox/{id}/unread", post(unread_message))
        .route("/api/inbox/{id}/dismiss", post(dismiss_inbox_message))
        .route("/api/inbox/{id}/undismiss", post(undismiss_inbox_message))
        .with_state(pool)
}
