use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;

use crate::envelope_validation::validate_envelope;
use crate::events::EventConsumer;
use crate::handlers::{handle_payment_collection_requested, EnvelopeMetadata};
use crate::models::PaymentCollectionRequestedPayload;

/// Start consumer task that subscribes to ar.payment.collection.requested events
///
/// This function spawns a background task that:
/// 1. Subscribes to AR payment collection events
/// 2. Processes payments using mock processor
/// 3. Emits payment succeeded events
pub async fn start_payment_collection_consumer(
    bus: Arc<dyn EventBus>,
    pool: PgPool,
) {
    tokio::spawn(async move {
        tracing::info!("Starting payment collection consumer");

        // Subscribe to AR payment collection events
        let subject = "ar.events.payment.collection.requested";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        tracing::info!("Subscribed to {}", subject);

        let consumer = EventConsumer::new(pool.clone());

        // Configure retry behavior: 3 attempts with exponential backoff
        let retry_config = RetryConfig::default();

        while let Some(msg) = stream.next().await {
            // Clone necessary data for retry closure
            let consumer_clone = consumer.clone();
            let pool_clone = pool.clone();
            let msg_clone = msg.clone();

            // Retry processing with exponential backoff
            // Wrap in a Send-safe error type (String)
            let result = retry_with_backoff(
                || {
                    let consumer = consumer_clone.clone();
                    let pool = pool_clone.clone();
                    let msg = msg_clone.clone();
                    async move {
                        process_payment_collection_request(&consumer, &pool, &msg)
                            .await
                            .map_err(|e| format!("{:#}", e))
                    }
                },
                &retry_config,
                "payments_collection_consumer",
            )
            .await;

            // If all retries failed, send to DLQ
            if let Err(error_msg) = result {
                crate::events::dlq::handle_processing_error(
                    &pool,
                    &msg,
                    &error_msg,
                    retry_config.max_attempts as i32,
                )
                .await;
            }
        }

        tracing::warn!("Payment collection consumer stopped");
    });
}

async fn process_payment_collection_request(
    consumer: &EventConsumer,
    pool: &PgPool,
    msg: &BusMessage,
) -> anyhow::Result<()> {
    // Use EventConsumer's idempotent processing
    consumer
        .process_idempotent(msg, |payload: PaymentCollectionRequestedPayload| async move {
            // Extract envelope metadata from the message
            let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;

            // Validate envelope fields first
            validate_envelope(&envelope)
                .map_err(|e| anyhow::anyhow!("Envelope validation failed: {}", e))?;

            // Extract metadata (validation ensures these fields exist and are valid)
            let event_id = envelope
                .get("event_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing event_id"))?;
            let event_id = uuid::Uuid::parse_str(event_id)?;

            let tenant_id = envelope
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing tenant_id"))?
                .to_string();

            // Accept both "correlation_id" (Payments envelope) and "trace_id" (AR envelope)
            let correlation_id = envelope
                .get("correlation_id")
                .or_else(|| envelope.get("trace_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let metadata = EnvelopeMetadata {
                event_id,
                tenant_id,
                correlation_id,
            };

            // Handle the payment collection request
            handle_payment_collection_requested(pool, payload, metadata).await
        })
        .await
}
