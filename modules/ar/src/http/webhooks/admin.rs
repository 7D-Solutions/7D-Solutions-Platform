use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use sqlx::PgPool;

use security::VerifiedClaims;

use crate::domain::webhooks as webhook_repo;
use crate::models::{
    ApiError, ListWebhooksQuery, PaginatedResponse, ReplayWebhookRequest, TilledWebhookEvent,
    Webhook, WebhookStatus,
};

use super::process_webhook_event;

#[utoipa::path(get, path = "/api/ar/webhooks", tag = "Webhooks",
    params(
        ("event_type" = Option<String>, Query, description = "Filter by event type"),
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("limit" = Option<i64>, Query, description = "Page size (max 100)"),
        ("offset" = Option<i64>, Query, description = "Offset"),
    ),
    responses((status = 200, description = "Paginated list of webhooks", body = PaginatedResponse<Webhook>)),
    security(("bearer" = [])))]
/// GET /api/ar/webhooks - List webhooks (admin)
pub async fn list_webhooks(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListWebhooksQuery>,
) -> Result<Json<PaginatedResponse<Webhook>>, ApiError> {
    let app_id = super::super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);
    let page = if limit > 0 { offset / limit + 1 } else { 1 };

    let total_items = webhook_repo::count_webhooks(
        &db,
        &app_id,
        query.event_type.as_deref(),
        query.status.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to count webhooks");
        ApiError::internal("Failed to count webhooks")
    })?;

    let webhooks = webhook_repo::list_webhooks(
        &db,
        &app_id,
        query.event_type.as_deref(),
        query.status.as_deref(),
        limit,
        offset,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to list webhooks");
        ApiError::internal("Failed to list webhooks")
    })?;

    Ok(Json(PaginatedResponse::new(
        webhooks,
        page.into(),
        limit.into(),
        total_items,
    )))
}

#[utoipa::path(get, path = "/api/ar/webhooks/{id}", tag = "Webhooks",
    params(("id" = i32, Path, description = "Webhook ID")),
    responses(
        (status = 200, description = "Webhook details", body = serde_json::Value),
        (status = 404, description = "Webhook not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
/// GET /api/ar/webhooks/:id - Get webhook details
pub async fn get_webhook(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Webhook>, ApiError> {
    let app_id = super::super::tenant::extract_tenant(&claims)?;

    let webhook = webhook_repo::fetch_by_id(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch webhook");
            ApiError::internal("Failed to fetch webhook")
        })?
        .ok_or_else(|| ApiError::not_found("Webhook not found"))?;

    Ok(Json(webhook))
}

#[utoipa::path(post, path = "/api/ar/webhooks/{id}/replay", tag = "Webhooks",
    params(("id" = i32, Path, description = "Webhook ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Webhook replayed"),
        (status = 400, description = "Replay not allowed", body = platform_http_contracts::ApiError),
        (status = 404, description = "Webhook not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
/// POST /api/ar/webhooks/:id/replay - Replay a webhook
pub async fn replay_webhook(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<ReplayWebhookRequest>,
) -> Result<StatusCode, ApiError> {
    let app_id = super::super::tenant::extract_tenant(&claims)?;

    let webhook = webhook_repo::fetch_by_id(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch webhook");
            ApiError::internal("Failed to fetch webhook")
        })?
        .ok_or_else(|| ApiError::not_found("Webhook not found"))?;

    let force = req.force.unwrap_or(false);
    if webhook.status != WebhookStatus::Failed && !force {
        return Err(ApiError::bad_request(
            "Can only replay failed webhooks (use force=true to override)",
        ));
    }

    let event: TilledWebhookEvent = serde_json::from_value(
        webhook
            .payload
            .ok_or_else(|| ApiError::bad_request("Webhook has no payload"))?,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to parse webhook payload");
        ApiError::internal("Failed to parse webhook payload")
    })?;

    webhook_repo::set_replay_processing(&db, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to update webhook status");
            ApiError::internal("Failed to update webhook status")
        })?;

    match process_webhook_event(&db, &app_id, &event).await {
        Ok(_) => {
            webhook_repo::set_replay_processed(&db, id).await.ok();
            tracing::info!("Successfully replayed webhook {}", id);
            Ok(StatusCode::OK)
        }
        Err(e) => {
            webhook_repo::set_failed(&db, id, &e).await.ok();
            tracing::error!(id = %id, error = %e, "Failed to replay webhook");
            Err(ApiError::internal(format!("Webhook replay failed: {}", e)))
        }
    }
}
