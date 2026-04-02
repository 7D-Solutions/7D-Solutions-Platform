use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use sqlx::PgPool;

use security::VerifiedClaims;

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

    // Count total matching rows
    let mut count_sql = String::from(
        "SELECT COUNT(*) as total FROM ar_webhooks WHERE app_id = $1",
    );
    let mut count_param = 1;

    if query.event_type.is_some() {
        count_param += 1;
        count_sql.push_str(&format!(" AND event_type = ${}", count_param));
    }
    if query.status.is_some() {
        count_param += 1;
        count_sql.push_str(&format!(
            " AND status = ${}::ar_webhooks_status",
            count_param
        ));
    }

    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(event_type) = &query.event_type {
        count_query = count_query.bind(event_type);
    }
    if let Some(status) = &query.status {
        count_query = count_query.bind(status);
    }
    let total_items = count_query.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Failed to count webhooks: {:?}", e);
        ApiError::internal("Failed to count webhooks")
    })?;

    // Fetch page of results
    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE app_id = $1
        "#,
    );

    let mut param_count = 1;

    if query.event_type.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND event_type = ${}", param_count));
    }

    if query.status.is_some() {
        param_count += 1;
        sql.push_str(&format!(
            " AND status = ${}::ar_webhooks_status",
            param_count
        ));
    }

    sql.push_str(" ORDER BY received_at DESC LIMIT $");
    param_count += 1;
    sql.push_str(&param_count.to_string());
    sql.push_str(" OFFSET $");
    param_count += 1;
    sql.push_str(&param_count.to_string());

    let mut query_builder = sqlx::query_as::<_, Webhook>(&sql).bind(&app_id);

    if let Some(event_type) = &query.event_type {
        query_builder = query_builder.bind(event_type);
    }

    if let Some(status) = &query.status {
        query_builder = query_builder.bind(status);
    }

    query_builder = query_builder.bind(limit).bind(offset);

    let webhooks = query_builder.fetch_all(&db).await.map_err(|e| {
        tracing::error!("Failed to list webhooks: {:?}", e);
        ApiError::internal("Failed to list webhooks")
    })?;

    Ok(Json(PaginatedResponse::new(webhooks, page.into(), limit.into(), total_items)))
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

    let webhook = sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch webhook: {:?}", e);
        ApiError::internal("Failed to fetch webhook")
    })?
    .ok_or_else(|| {
        ApiError::not_found("Webhook not found")
    })?;

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

    // Fetch webhook
    let webhook = sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch webhook: {:?}", e);
        ApiError::internal("Failed to fetch webhook")
    })?
    .ok_or_else(|| {
        ApiError::not_found("Webhook not found")
    })?;

    // Check if replay is allowed
    let force = req.force.unwrap_or(false);
    if webhook.status != WebhookStatus::Failed && !force {
        return Err(ApiError::bad_request(
            "Can only replay failed webhooks (use force=true to override)",
        ));
    }

    // Parse payload
    let event: TilledWebhookEvent = serde_json::from_value(webhook.payload.ok_or_else(|| {
        ApiError::bad_request("Webhook has no payload")
    })?)
    .map_err(|e| {
        tracing::error!("Failed to parse webhook payload: {}", e);
        ApiError::internal("Failed to parse webhook payload")
    })?;

    // Update status to processing
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW(), attempt_count = attempt_count + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update webhook status: {}", e);
        ApiError::internal("Failed to update webhook status")
    })?;

    // Process the event
    match process_webhook_event(&db, &app_id, &event).await {
        Ok(_) => {
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'processed', processed_at = NOW(), error = NULL, error_code = NULL
                WHERE id = $1
                "#,
            )
            .bind(id)
            .execute(&db)
            .await
            .ok();

            tracing::info!("Successfully replayed webhook {}", id);
            Ok(StatusCode::OK)
        }
        Err(e) => {
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'failed', error = $1, error_code = 'processing_error'
                WHERE id = $2
                "#,
            )
            .bind(&e)
            .bind(id)
            .execute(&db)
            .await
            .ok();

            tracing::error!("Failed to replay webhook {}: {}", id, e);
            Err(ApiError::internal(format!("Webhook replay failed: {}", e)))
        }
    }
}
