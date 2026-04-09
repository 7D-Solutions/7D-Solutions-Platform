use axum::Router;
use std::sync::Arc;
use std::time::Duration;

use notifications_rs::{
    config::Config,
    consumers::{
        shipping::{
            handle_outbound_delivered, handle_outbound_shipped, OutboundDeliveredPayload,
            OutboundShippedPayload,
        },
        EventConsumer,
    },
    handlers::{handle_invoice_issued, handle_payment_failed, handle_payment_succeeded},
    http, metrics,
    models::{
        EnvelopeMetadata, InvoiceIssuedPayload, PaymentFailedPayload, PaymentSucceededPayload,
    },
    scheduled::{
        dispatch_once, reset_orphaned_claims, ChannelRouter, HttpEmailSender, HttpSmsSender,
        LoggingSender, NotificationSender, RetryPolicy, SendGridEmailSender,
    },
};
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};
use security::{permissions, RequirePermissionsLayer};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer("ar.events.ar.invoice_opened", on_invoice_issued)
        .consumer(
            "payments.events.payment.succeeded",
            on_payment_succeeded,
        )
        .consumer("payments.events.payment.failed", on_payment_failed)
        .consumer(
            "shipping_receiving.outbound_shipped",
            on_outbound_shipped,
        )
        .consumer(
            "shipping_receiving.outbound_delivered",
            on_outbound_delivered,
        )
        .routes(|ctx| {
            let pool = ctx.pool().clone();
            let config = Config::from_env().unwrap_or_else(|err| {
                tracing::error!("Notifications config error: {}", err);
                panic!("Notifications config error: {}", err);
            });

            // Startup: recover orphaned claimed notifications
            {
                let startup_pool = pool.clone();
                tokio::spawn(async move {
                    use chrono::Utc;
                    let cutoff = Utc::now() - chrono::Duration::minutes(5);
                    match reset_orphaned_claims(&startup_pool, cutoff).await {
                        Ok(n) if n > 0 => {
                            tracing::warn!(count = n, "reset orphaned claimed notifications")
                        }
                        Ok(_) => tracing::debug!("no orphaned claimed notifications found"),
                        Err(e) => tracing::error!(error = %e, "failed to reset orphaned claims"),
                    }
                });
            }

            // Spawn background notification dispatcher loop
            {
                let interval_secs: u64 = std::env::var("NOTIFICATIONS_DISPATCH_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60);
                let dispatch_pool = pool.clone();
                let retry_policy = RetryPolicy {
                    max_attempts: config.retry_max_attempts,
                    backoff_base_secs: config.retry_backoff_base_secs,
                    backoff_multiplier: config.retry_backoff_multiplier,
                    backoff_max_secs: config.retry_backoff_max_secs,
                };
                let email_sender: Arc<dyn NotificationSender> =
                    match config.email_sender_type {
                        notifications_rs::config::EmailSenderType::Logging => {
                            Arc::new(LoggingSender)
                        }
                        notifications_rs::config::EmailSenderType::Http => {
                            Arc::new(HttpEmailSender::new(
                                config
                                    .email_http_endpoint
                                    .clone()
                                    .expect("EMAIL_HTTP_ENDPOINT required for HTTP sender"),
                                config.email_from.clone(),
                                config.email_api_key.clone(),
                            ))
                        }
                        notifications_rs::config::EmailSenderType::SendGrid => {
                            Arc::new(SendGridEmailSender::new(
                                config.email_from.clone(),
                                config
                                    .sendgrid_api_key
                                    .clone()
                                    .expect("SENDGRID_API_KEY required for SendGrid sender"),
                            ))
                        }
                    };
                let sms_sender: Arc<dyn NotificationSender> =
                    match config.sms_sender_type {
                        notifications_rs::config::SmsSenderType::Logging => {
                            Arc::new(LoggingSender)
                        }
                        notifications_rs::config::SmsSenderType::Http => {
                            Arc::new(HttpSmsSender::new(
                                config
                                    .sms_http_endpoint
                                    .clone()
                                    .expect("SMS_HTTP_ENDPOINT required for HTTP SMS sender"),
                                config.sms_from_number.clone(),
                                config.sms_api_key.clone(),
                            ))
                        }
                    };
                let dispatch_sender: Arc<dyn NotificationSender> =
                    Arc::new(ChannelRouter {
                        email: email_sender,
                        sms: sms_sender,
                    });
                tokio::spawn(async move {
                    loop {
                        if let Err(e) =
                            dispatch_once(&dispatch_pool, dispatch_sender.clone(), retry_policy)
                                .await
                        {
                            tracing::error!(error = %e, "dispatch_once error");
                        }
                        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                    }
                });
            }

            // Register SLO metrics with global prometheus registry so
            // SDK's /metrics endpoint picks them up via prometheus::gather().
            let _ = prometheus::register(Box::new(
                metrics::HTTP_REQUEST_DURATION_SECONDS.clone(),
            ));
            let _ = prometheus::register(Box::new(
                metrics::HTTP_REQUESTS_TOTAL.clone(),
            ));
            let _ = prometheus::register(Box::new(
                metrics::EVENT_CONSUMER_LAG_MESSAGES.clone(),
            ));

            Router::new()
                .merge(
                    http::admin::admin_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_MUTATE]),
                    ),
                )
                .merge(
                    http::dlq::dlq_read_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_READ]),
                    ),
                )
                .merge(
                    http::dlq::dlq_mutate_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_MUTATE]),
                    ),
                )
                .merge(
                    http::inbox::inbox_read_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_READ]),
                    ),
                )
                .merge(
                    http::inbox::inbox_mutate_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_MUTATE]),
                    ),
                )
                .merge(
                    http::templates::templates_read_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_READ]),
                    ),
                )
                .merge(
                    http::templates::templates_mutate_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_MUTATE]),
                    ),
                )
                .merge(
                    http::sends::sends_read_router(pool.clone()).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_READ]),
                    ),
                )
                .merge(
                    http::sends::sends_mutate_router(pool).route_layer(
                        RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_MUTATE]),
                    ),
                )
        })
        .run()
        .await
        .expect("notifications module failed");
}

/// SDK consumer adapter for ar.events.ar.invoice_opened.
async fn on_invoice_issued(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let consumer = EventConsumer::new(pool.clone());
    if consumer
        .is_processed(event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate invoice_opened event ignored");
        return Ok(());
    }

    let payload: InvoiceIssuedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: envelope.tenant_id.clone(),
        correlation_id: envelope.correlation_id.clone(),
    };

    handle_invoice_issued(pool, payload, metadata)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    consumer
        .mark_processed(
            event_id,
            "ar.events.ar.invoice_opened",
            &envelope.tenant_id,
            &envelope.source_module,
        )
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}

/// SDK consumer adapter for payments.events.payment.succeeded.
async fn on_payment_succeeded(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let consumer = EventConsumer::new(pool.clone());
    if consumer
        .is_processed(event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate payment.succeeded event ignored");
        return Ok(());
    }

    let payload: PaymentSucceededPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: envelope.tenant_id.clone(),
        correlation_id: envelope.correlation_id.clone(),
    };

    handle_payment_succeeded(pool, payload, metadata)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    consumer
        .mark_processed(
            event_id,
            "payments.events.payment.succeeded",
            &envelope.tenant_id,
            &envelope.source_module,
        )
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}

/// SDK consumer adapter for shipping_receiving.outbound_shipped.
async fn on_outbound_shipped(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let consumer = EventConsumer::new(pool.clone());
    if consumer
        .is_processed(event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate outbound_shipped event ignored");
        return Ok(());
    }

    let payload: OutboundShippedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: envelope.tenant_id.clone(),
        correlation_id: envelope.correlation_id.clone(),
    };

    handle_outbound_shipped(pool, payload, metadata)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    consumer
        .mark_processed(
            event_id,
            "shipping_receiving.outbound_shipped",
            &envelope.tenant_id,
            &envelope.source_module,
        )
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}

/// SDK consumer adapter for shipping_receiving.outbound_delivered.
async fn on_outbound_delivered(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let consumer = EventConsumer::new(pool.clone());
    if consumer
        .is_processed(event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate outbound_delivered event ignored");
        return Ok(());
    }

    let payload: OutboundDeliveredPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: envelope.tenant_id.clone(),
        correlation_id: envelope.correlation_id.clone(),
    };

    handle_outbound_delivered(pool, payload, metadata)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    consumer
        .mark_processed(
            event_id,
            "shipping_receiving.outbound_delivered",
            &envelope.tenant_id,
            &envelope.source_module,
        )
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}

/// SDK consumer adapter for payments.events.payment.failed.
async fn on_payment_failed(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let consumer = EventConsumer::new(pool.clone());
    if consumer
        .is_processed(event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate payment.failed event ignored");
        return Ok(());
    }

    let payload: PaymentFailedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: envelope.tenant_id.clone(),
        correlation_id: envelope.correlation_id.clone(),
    };

    handle_payment_failed(pool, payload, metadata)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    consumer
        .mark_processed(
            event_id,
            "payments.events.payment.failed",
            &envelope.tenant_id,
            &envelope.source_module,
        )
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}
