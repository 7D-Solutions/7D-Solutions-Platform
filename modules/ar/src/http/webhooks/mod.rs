pub mod admin;
mod customer;
mod payment_methods;
mod payments;
mod signature;
mod subscriptions;

pub use admin::{get_webhook, list_webhooks, replay_webhook};

use axum::{body::Bytes, http::HeaderMap, http::StatusCode};

use axum::extract::State;
use sqlx::PgPool;

use crate::domain::webhooks as webhook_repo;
use crate::models::{ApiError, TilledWebhookEvent};

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
            tracing::warn!(event_type = %event.event_type, "Unhandled webhook event type");
        }
    }

    Ok(())
}

#[utoipa::path(post, path = "/api/ar/webhooks/tilled", tag = "Webhooks",
    responses(
        (status = 200, description = "Webhook received and processed"),
        (status = 401, description = "Signature verification failed", body = platform_http_contracts::ApiError),
    ))]
/// POST /api/ar/webhooks/tilled - Receive Tilled webhook
pub async fn receive_tilled_webhook(
    State(db): State<PgPool>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let app_id = std::env::var("TILLED_WEBHOOK_APP_ID").unwrap_or_else(|_| {
        headers
            .get("x-tilled-account")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string()
    });

    let webhook_secret = std::env::var("TILLED_WEBHOOK_SECRET_TRASHTECH")
        .or_else(|_| std::env::var("TILLED_WEBHOOK_SECRET"))
        .map_err(|_| {
            tracing::error!(error_code = "OPERATION_FAILED", "TILLED_WEBHOOK_SECRET not configured — rejecting webhook");
            ApiError::internal("Webhook secret not configured")
        })?;

    let sig = headers
        .get("tilled-signature")
        .or_else(|| headers.get("x-tilled-signature"))
        .and_then(|v| v.to_str().ok());

    if let Err(e) = signature::verify_tilled_signature(&body, sig, &webhook_secret) {
        tracing::warn!(error = %e, "Webhook signature verification failed");
        return Err(ApiError::unauthorized(e));
    }

    let event: TilledWebhookEvent = serde_json::from_slice(&body).map_err(|e| {
        tracing::error!(error = %e, "Failed to parse webhook event");
        ApiError::bad_request(format!("Failed to parse webhook: {}", e))
    })?;

    tracing::info!(
        "Received webhook event: {} (id: {})",
        event.event_type,
        event.id
    );

    // Check for duplicate event (idempotency)
    let existing = webhook_repo::check_duplicate_event(&db, &event.id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to check for duplicate webhook");
            ApiError::internal("Failed to check idempotency")
        })?;

    if existing.is_some() {
        tracing::info!("Webhook event {} already processed (idempotent)", event.id);
        return Ok(StatusCode::OK);
    }

    // Store webhook in database (status: received)
    let webhook_id = webhook_repo::insert_webhook(
        &db,
        &app_id,
        &event.id,
        &event.event_type,
        serde_json::to_value(&event).map_err(|e| {
            tracing::error!(error = %e, "Failed to serialize webhook event");
            ApiError::internal("Failed to serialize event")
        })?,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to store webhook");
        ApiError::internal("Failed to store webhook")
    })?;

    // Update status to processing
    webhook_repo::set_processing(&db, webhook_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to update webhook status");
            ApiError::internal("Failed to update webhook status")
        })?;

    // Process the event
    match process_webhook_event(&db, &app_id, &event).await {
        Ok(_) => {
            webhook_repo::set_processed(&db, webhook_id).await.ok();
            tracing::info!("Successfully processed webhook event {}", event.id);
        }
        Err(e) => {
            webhook_repo::set_failed(&db, webhook_id, &e).await.ok();
            tracing::error!(id = %event.id, error = %e, "Failed to process webhook event");
        }
    }

    Ok(StatusCode::OK)
}
