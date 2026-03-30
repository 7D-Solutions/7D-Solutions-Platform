use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use sqlx::PgPool;

use security::VerifiedClaims;

use crate::models::{
    ErrorResponse, ListWebhooksQuery, ReplayWebhookRequest, TilledWebhookEvent, Webhook,
    WebhookStatus,
};

use super::process_webhook_event;

/// GET /api/ar/webhooks - List webhooks (admin)
pub async fn list_webhooks(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListWebhooksQuery>,
) -> Result<Json<Vec<Webhook>>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to list webhooks",
            )),
        )
    })?;

    Ok(Json(webhooks))
}

/// GET /api/ar/webhooks/:id - Get webhook details
pub async fn get_webhook(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Webhook>, (StatusCode, Json<ErrorResponse>)> {
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch webhook",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Webhook not found")),
        )
    })?;

    Ok(Json(webhook))
}

/// POST /api/ar/webhooks/:id/replay - Replay a webhook
pub async fn replay_webhook(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<ReplayWebhookRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch webhook",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Webhook not found")),
        )
    })?;

    // Check if replay is allowed
    let force = req.force.unwrap_or(false);
    if webhook.status != WebhookStatus::Failed && !force {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_status",
                "Can only replay failed webhooks (use force=true to override)",
            )),
        ));
    }

    // Parse payload
    let event: TilledWebhookEvent = serde_json::from_value(webhook.payload.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_webhook",
                "Webhook has no payload",
            )),
        )
    })?)
    .map_err(|e| {
        tracing::error!("Failed to parse webhook payload: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "parse_error",
                "Failed to parse webhook payload",
            )),
        )
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update webhook status",
            )),
        )
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
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("processing_error", e)),
            ))
        }
    }
}
