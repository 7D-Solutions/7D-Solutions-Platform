mod admin;
mod customer;
mod payment_methods;
mod payments;
mod signature;
mod subscriptions;

pub use admin::{get_webhook, list_webhooks, replay_webhook};

use axum::{body::Bytes, http::HeaderMap, http::StatusCode, Json};

use axum::extract::State;
use sqlx::PgPool;

use crate::models::{ErrorResponse, TilledWebhookEvent};

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
            customer::process_customer_event(db, app_id, event).await?;
        }
        // Payment intent events
        "payment_intent.succeeded" | "payment_intent.failed" => {
            payments::process_payment_intent_event(db, app_id, event).await?;
        }
        // Payment method events
        "payment_method.attached" | "payment_method.detached" => {
            payment_methods::process_payment_method_event(db, app_id, event).await?;
        }
        // Subscription events
        "subscription.created" | "subscription.updated" | "subscription.canceled" => {
            subscriptions::process_subscription_event(db, app_id, event).await?;
        }
        // Charge events
        "charge.succeeded" | "charge.failed" | "charge.refunded" => {
            payments::process_charge_event(db, app_id, event).await?;
        }
        // Invoice events
        "invoice.created" | "invoice.payment_succeeded" | "invoice.payment_failed" => {
            payments::process_invoice_event(db, app_id, event).await?;
        }
        _ => {
            tracing::warn!("Unhandled webhook event type: {}", event.event_type);
        }
    }

    Ok(())
}

/// POST /api/ar/webhooks/tilled - Receive Tilled webhook
pub async fn receive_tilled_webhook(
    State(db): State<PgPool>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Webhook endpoints are called by Tilled (HMAC-authenticated, not JWT).
    // Tenant is determined by the registered webhook endpoint configuration.
    let app_id = std::env::var("TILLED_WEBHOOK_APP_ID").unwrap_or_else(|_| {
        headers
            .get("x-tilled-account")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string()
    });

    // Get webhook secret from environment — required, no fallback
    let webhook_secret = std::env::var("TILLED_WEBHOOK_SECRET_TRASHTECH")
        .or_else(|_| std::env::var("TILLED_WEBHOOK_SECRET"))
        .map_err(|_| {
            tracing::error!("TILLED_WEBHOOK_SECRET not configured — rejecting webhook");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "config_error",
                    "Webhook secret not configured",
                )),
            )
        })?;

    // Always verify signature — no bypass
    let sig = headers
        .get("tilled-signature")
        .or_else(|| headers.get("x-tilled-signature"))
        .and_then(|v| v.to_str().ok());

    if let Err(e) = signature::verify_tilled_signature(&body, sig, &webhook_secret) {
        tracing::warn!("Webhook signature verification failed: {}", e);
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new("signature_error", e)),
        ));
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
    .bind(&app_id)
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
    .bind(&app_id)
    .bind(&event.id)
    .bind(&event.event_type)
    .bind(serde_json::to_value(&event).map_err(|e| {
        tracing::error!("Failed to serialize webhook event: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "serialization_error",
                "Failed to serialize event",
            )),
        )
    })?)
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
    match process_webhook_event(&db, &app_id, &event).await {
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
