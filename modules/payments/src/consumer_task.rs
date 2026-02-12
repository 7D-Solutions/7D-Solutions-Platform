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
        let subject = "ar.events.ar.payment.collection.requested";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        tracing::info!("Subscribed to {}", subject);

        let consumer = EventConsumer::new(pool.clone());

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_payment_collection_request(&consumer, &pool, &msg).await {
                tracing::error!(
                    subject = %msg.subject,
                    error = %e,
                    "Failed to process payment collection request"
                );
            }
        }

        tracing::warn!("Payment collection consumer stopped");
    });
}

async fn process_payment_collection_request(
    consumer: &EventConsumer,
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use EventConsumer's idempotent processing
    consumer
        .process_idempotent(msg, |payload: PaymentCollectionRequestedPayload| async {
            // Extract envelope metadata from the message
            let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;

            // Validate envelope fields first
            validate_envelope(&envelope)
                .map_err(|e| format!("Envelope validation failed: {}", e))?;

            // Extract metadata (validation ensures these fields exist and are valid)
            let event_id = envelope
                .get("event_id")
                .and_then(|v| v.as_str())
                .ok_or("Missing event_id")?;
            let event_id = uuid::Uuid::parse_str(event_id)?;

            let tenant_id = envelope
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .ok_or("Missing tenant_id")?
                .to_string();

            let correlation_id = envelope
                .get("correlation_id")
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
