use ar_rs::{consumer_tasks, events, http, metrics, models::PaymentSucceededPayload};
use axum::Extension;
use std::sync::Arc;
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer(
            "payments.events.payment.succeeded",
            on_payment_succeeded,
        )
        .routes(|ctx| {
            // Register AR prometheus metrics with the global registry.
            // The SDK's /metrics endpoint uses prometheus::gather() which
            // picks these up automatically.
            let _ar_metrics =
                metrics::ArMetrics::new().expect("AR: failed to create metrics");

            let party_client = Arc::new(
                ctx.platform_client::<platform_client_party::PartiesClient>(),
            );

            http::ar_router(ctx.pool().clone())
                .merge(http::tax::tax_router(ctx.pool().clone()))
                .merge(http::admin::admin_router(ctx.pool().clone()))
                .layer(Extension(party_client))
        })
        .run()
        .await
        .expect("ar module failed");
}

/// SDK consumer handler for payments.events.payment.succeeded.
///
/// Adapts the existing `consumer_tasks::handle_payment_succeeded` business logic
/// to the SDK consumer handler signature.
///
/// NOTE: The SDK consumer provides retry-with-backoff but does NOT send failed
/// events to the DLQ after exhausting retries (the old hand-rolled consumer did).
/// This gap is documented for the API freeze review (bd-521mv).
async fn on_payment_succeeded(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    // Idempotency check
    if events::is_event_processed(pool, event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate payment.succeeded event ignored");
        return Ok(());
    }

    // Extract payload from typed envelope
    let payload: PaymentSucceededPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    tracing::info!(
        event_id = %event_id,
        invoice_id = %payload.invoice_id,
        payment_id = %payload.payment_id,
        amount = %payload.amount_minor,
        "Processing payment.succeeded event"
    );

    // Business logic: mark invoice paid + emit ar.invoice_paid outbox event
    consumer_tasks::handle_payment_succeeded(pool, &payload)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    // Mark event as processed for idempotency
    events::mark_event_processed(
        pool,
        event_id,
        "payments.payment.succeeded",
        "ar-payment-consumer",
    )
    .await
    .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    tracing::info!(
        event_id = %event_id,
        invoice_id = %payload.invoice_id,
        "Payment successfully applied to invoice"
    );

    Ok(())
}
