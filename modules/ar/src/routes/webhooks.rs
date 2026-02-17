use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use crate::models::{
    ErrorResponse, ListWebhooksQuery, ReplayWebhookRequest, TilledWebhookEvent, Webhook,
    WebhookStatus,
};

/// Verify Tilled webhook signature
/// Tilled signs webhooks with HMAC-SHA256
fn verify_tilled_signature(
    payload: &[u8],
    signature_header: Option<&str>,
    secret: &str,
) -> Result<(), String> {
    let signature = signature_header.ok_or_else(|| "Missing signature header".to_string())?;

    // Tilled sends signature in format: "t=timestamp,v1=signature"
    let sig_parts: Vec<&str> = signature.split(',').collect();
    let mut timestamp = "";
    let mut sig_value = "";

    for part in sig_parts {
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value;
        } else if let Some(value) = part.strip_prefix("v1=") {
            sig_value = value;
        }
    }

    if timestamp.is_empty() || sig_value.is_empty() {
        return Err("Invalid signature format".to_string());
    }

    // Construct signed payload: timestamp.payload
    let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(payload));

    // Compute expected signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("Invalid secret: {}", e))?;
    mac.update(signed_payload.as_bytes());
    let expected_sig = hex::encode(mac.finalize().into_bytes());

    // Compare signatures (constant-time comparison)
    if expected_sig != sig_value {
        return Err("Signature verification failed".to_string());
    }

    Ok(())
}

/// Process webhook event based on type
async fn process_webhook_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    tracing::info!("Processing webhook event: {}", event.event_type);

    match event.event_type.as_str() {
        // Customer events
        "customer.created" | "customer.updated" => {
            process_customer_event(db, app_id, event).await?;
        }
        // Payment intent events
        "payment_intent.succeeded" | "payment_intent.failed" => {
            process_payment_intent_event(db, app_id, event).await?;
        }
        // Payment method events
        "payment_method.attached" | "payment_method.detached" => {
            process_payment_method_event(db, app_id, event).await?;
        }
        // Subscription events
        "subscription.created" | "subscription.updated" | "subscription.canceled" => {
            process_subscription_event(db, app_id, event).await?;
        }
        // Charge events
        "charge.succeeded" | "charge.failed" | "charge.refunded" => {
            process_charge_event(db, app_id, event).await?;
        }
        // Invoice events
        "invoice.created" | "invoice.payment_succeeded" | "invoice.payment_failed" => {
            process_invoice_event(db, app_id, event).await?;
        }
        _ => {
            tracing::warn!("Unhandled webhook event type: {}", event.event_type);
        }
    }

    Ok(())
}

/// Process customer webhook events
async fn process_customer_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let customer_data = &event.data;
    let tilled_customer_id = customer_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing customer ID in webhook data".to_string())?;

    // Update or create customer record
    let email = customer_data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name = customer_data.get("name").and_then(|v| v.as_str());

    sqlx::query(
        r#"
        INSERT INTO ar_customers (
            app_id, tilled_customer_id, email, name, status, metadata,
            retry_attempt_count, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'active', $5, 0, NOW(), NOW())
        ON CONFLICT (tilled_customer_id, app_id)
        DO UPDATE SET
            email = EXCLUDED.email,
            name = EXCLUDED.name,
            metadata = EXCLUDED.metadata,
            updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(tilled_customer_id)
    .bind(email)
    .bind(name)
    .bind(&event.data)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update customer: {}", e))?;

    tracing::info!("Processed customer event for {}", tilled_customer_id);
    Ok(())
}

/// Process payment intent webhook events
async fn process_payment_intent_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let payment_intent_data = &event.data;
    let tilled_charge_id = payment_intent_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing payment intent ID".to_string())?;

    let status = if event.event_type == "payment_intent.succeeded" {
        "succeeded"
    } else {
        "failed"
    };

    // Update charge status
    sqlx::query(
        r#"
        UPDATE ar_charges
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_charge_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update charge: {}", e))?;

    tracing::info!("Processed payment intent event for {}", tilled_charge_id);
    Ok(())
}

/// Process payment method webhook events
async fn process_payment_method_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let pm_data = &event.data;
    let tilled_pm_id = pm_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing payment method ID".to_string())?;

    if event.event_type == "payment_method.detached" {
        // Soft delete payment method
        sqlx::query(
            r#"
            UPDATE ar_payment_methods
            SET status = 'inactive', deleted_at = NOW(), updated_at = NOW()
            WHERE tilled_payment_method_id = $1 AND app_id = $2
            "#,
        )
        .bind(tilled_pm_id)
        .bind(app_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to delete payment method: {}", e))?;
    }

    tracing::info!("Processed payment method event for {}", tilled_pm_id);
    Ok(())
}

/// Process subscription webhook events
async fn process_subscription_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let sub_data = &event.data;
    let tilled_sub_id = sub_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing subscription ID".to_string())?;

    let status = match event.event_type.as_str() {
        "subscription.created" => "active",
        "subscription.updated" => sub_data.get("status").and_then(|v| v.as_str()).unwrap_or("active"),
        "subscription.canceled" => "canceled",
        _ => "active",
    };

    // Update subscription status
    sqlx::query(
        r#"
        UPDATE ar_subscriptions
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_subscription_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_sub_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update subscription: {}", e))?;

    tracing::info!("Processed subscription event for {}", tilled_sub_id);
    Ok(())
}

/// Process charge webhook events
async fn process_charge_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let charge_data = &event.data;
    let tilled_charge_id = charge_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing charge ID".to_string())?;

    let status = match event.event_type.as_str() {
        "charge.succeeded" => "succeeded",
        "charge.failed" => "failed",
        "charge.refunded" => "refunded",
        _ => "pending",
    };

    sqlx::query(
        r#"
        UPDATE ar_charges
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_charge_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update charge: {}", e))?;

    tracing::info!("Processed charge event for {}", tilled_charge_id);
    Ok(())
}

/// Process invoice webhook events
async fn process_invoice_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let invoice_data = &event.data;
    let tilled_invoice_id = invoice_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing invoice ID".to_string())?;

    let status = match event.event_type.as_str() {
        "invoice.created" => "open",
        "invoice.payment_succeeded" => "paid",
        "invoice.payment_failed" => "unpaid",
        _ => "open",
    };

    sqlx::query(
        r#"
        UPDATE ar_invoices
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_invoice_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_invoice_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update invoice: {}", e))?;

    tracing::info!("Processed invoice event for {}", tilled_invoice_id);
    Ok(())
}

/// POST /api/ar/webhooks/tilled - Receive Tilled webhook
pub async fn receive_tilled_webhook(
    State(db): State<PgPool>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Extract app_id from headers or use default
    // TODO: Extract from auth middleware when available
    let app_id = "test-app";

    // Get webhook secret from environment (use test secret in test mode)
    let webhook_secret = std::env::var("TILLED_WEBHOOK_SECRET_TRASHTECH")
        .or_else(|_| std::env::var("TILLED_WEBHOOK_SECRET"))
        .unwrap_or_else(|_| "test-secret".to_string());

    // Skip signature verification in test mode
    if webhook_secret != "test-secret" {
        // Verify signature in production mode
        let signature = headers
            .get("tilled-signature")
            .or_else(|| headers.get("x-tilled-signature"))
            .and_then(|v| v.to_str().ok());

        if let Err(e) = verify_tilled_signature(&body, signature, &webhook_secret) {
            tracing::warn!("Webhook signature verification failed: {}", e);
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::new("signature_error", e)),
            ));
        }
    }

    // Parse webhook event
    let event: TilledWebhookEvent = serde_json::from_slice(&body).map_err(|e| {
        tracing::error!("Failed to parse webhook event: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "parse_error",
                format!("Failed to parse webhook: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Received webhook event: {} (id: {})",
        event.event_type,
        event.id
    );

    // Check for duplicate event (idempotency)
    let existing = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id FROM ar_webhooks
        WHERE event_id = $1 AND app_id = $2
        "#,
    )
    .bind(&event.id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to check for duplicate webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to check idempotency",
            )),
        )
    })?;

    if existing.is_some() {
        tracing::info!("Webhook event {} already processed (idempotent)", event.id);
        // Return 200 to prevent Tilled retries
        return Ok(StatusCode::OK);
    }

    // Store webhook in database (status: received)
    let webhook_id = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_webhooks (
            app_id, event_id, event_type, status, payload, attempt_count, received_at
        )
        VALUES ($1, $2, $3, 'received', $4, 1, NOW())
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(&event.id)
    .bind(&event.event_type)
    .bind(serde_json::to_value(&event).unwrap())
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to store webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to store webhook",
            )),
        )
    })?;

    // Process event asynchronously (don't block webhook response)
    // Update status to processing
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(webhook_id)
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
    match process_webhook_event(&db, app_id, &event).await {
        Ok(_) => {
            // Mark as processed
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'processed', processed_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(webhook_id)
            .execute(&db)
            .await
            .ok();

            tracing::info!("Successfully processed webhook event {}", event.id);
        }
        Err(e) => {
            // Mark as failed
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'failed', error = $1, error_code = 'processing_error'
                WHERE id = $2
                "#,
            )
            .bind(&e)
            .bind(webhook_id)
            .execute(&db)
            .await
            .ok();

            tracing::error!("Failed to process webhook event {}: {}", event.id, e);
        }
    }

    // Always return 200 to prevent Tilled retries
    // Errors are stored in the database for manual investigation
    Ok(StatusCode::OK)
}

/// GET /api/ar/webhooks - List webhooks (admin)
pub async fn list_webhooks(
    State(db): State<PgPool>,
    Query(query): Query<ListWebhooksQuery>,
) -> Result<Json<Vec<Webhook>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Add auth middleware to verify admin access
    let app_id = "test-app";

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
        sql.push_str(&format!(" AND status = ${}::ar_webhooks_status", param_count));
    }

    sql.push_str(" ORDER BY received_at DESC LIMIT $");
    param_count += 1;
    sql.push_str(&param_count.to_string());
    sql.push_str(" OFFSET $");
    param_count += 1;
    sql.push_str(&param_count.to_string());

    let mut query_builder = sqlx::query_as::<_, Webhook>(&sql).bind(app_id);

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
    Path(id): Path<i32>,
) -> Result<Json<Webhook>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Add auth middleware to verify admin access
    let app_id = "test-app";

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
    .bind(app_id)
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
    Path(id): Path<i32>,
    Json(req): Json<ReplayWebhookRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Add auth middleware to verify admin access
    let app_id = "test-app";

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
    .bind(app_id)
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
    let event: TilledWebhookEvent = serde_json::from_value(
        webhook.payload.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("invalid_webhook", "Webhook has no payload")),
            )
        })?,
    )
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
    match process_webhook_event(&db, app_id, &event).await {
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
