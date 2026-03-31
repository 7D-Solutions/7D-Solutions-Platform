//! HTTP endpoints for versioned notification templates.
//!
//!   POST /api/templates       — publish a new template version
//!   GET  /api/templates/{key} — resolve latest + version history

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{create_notifications_envelope, enqueue_event};
use crate::template_store::{models, repo};

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TemplateResponse {
    pub id: Uuid,
    pub template_key: String,
    pub version: i32,
    pub channel: String,
    pub subject: String,
    pub body: String,
    pub required_vars: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct TemplateDetailResponse {
    pub latest: TemplateResponse,
    pub versions: Vec<models::TemplateVersionSummary>,
}

// ── Handlers ────────────────────────────────────────────────────────

async fn publish_template(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(input): Json<models::CreateTemplate>,
) -> Result<(StatusCode, Json<TemplateResponse>), ApiError> {
    let tenant_id = require_tenant(&claims)?;
    let created_by = claims.as_ref().map(|Extension(c)| c.user_id.to_string());

    let tpl = repo::publish_template(&pool, &tenant_id, &input, created_by.as_deref())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Emit template.published event via outbox
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let event_payload = serde_json::json!({
        "template_key": tpl.template_key,
        "version": tpl.version,
        "channel": tpl.channel,
    });
    let envelope = create_notifications_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        "notifications.events.template.published".to_string(),
        None,
        None,
        "ADMINISTRATIVE".to_string(),
        event_payload,
    );
    enqueue_event(
        &mut tx,
        "notifications.events.template.published",
        &envelope,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(to_response(&tpl))))
}

async fn get_template(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(key): Path<String>,
) -> Result<Json<TemplateDetailResponse>, ApiError> {
    let tenant_id = require_tenant(&claims)?;

    let latest = repo::get_latest(&pool, &tenant_id, &key)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Template not found"))?;

    let versions = repo::list_versions(&pool, &tenant_id, &key)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(TemplateDetailResponse {
        latest: to_response(&latest),
        versions,
    }))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn to_response(tpl: &crate::template_store::models::NotificationTemplate) -> TemplateResponse {
    TemplateResponse {
        id: tpl.id,
        template_key: tpl.template_key.clone(),
        version: tpl.version,
        channel: tpl.channel.clone(),
        subject: tpl.subject.clone(),
        body: tpl.body.clone(),
        required_vars: tpl.required_vars.clone(),
        created_at: tpl.created_at,
    }
}

fn require_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized("Missing or invalid authentication")),
    }
}

// ── Router ──────────────────────────────────────────────────────────

pub fn templates_read_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/templates/{key}", get(get_template))
        .with_state(pool)
}

pub fn templates_mutate_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/templates", post(publish_template))
        .with_state(pool)
}
