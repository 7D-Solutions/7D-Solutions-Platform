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

        while let Some(msg) = stream.next().await {
            // Process the request
            let result = process_payment_collection_request(&consumer, &pool, &msg).await;

            // Handle errors by writing to DLQ
            if let Err(e) = result {
                // Convert error to string and drop e immediately
                let error_msg = format!("{:#}", e);
                drop(e);

                // Extract event_id, tenant_id, and envelope for DLQ
                let envelope: Result<serde_json::Value, _> = serde_json::from_slice(&msg.payload);

                match envelope {
                    Ok(env) => {
                        let event_id_opt = env.get("event_id")
                            .and_then(|v| v.as_str())
                            .and_then(|s| uuid::Uuid::parse_str(s).ok());

                        let tenant_id_opt = env.get("tenant_id")
                            .and_then(|v| v.as_str());

                        if let (Some(event_id), Some(tenant_id)) = (event_id_opt, tenant_id_opt) {
                            // Write to DLQ
                            if let Err(dlq_err) = crate::events::dlq::insert_failed_event(
                                &pool,
                                event_id,
                                &msg.subject,
                                tenant_id,
                                &env,
                                &error_msg,
                                0,
                            ).await {
                                tracing::error!(
                                    event_id = %event_id,
                                    subject = %msg.subject,
                                    tenant_id = %tenant_id,
                                    error = %error_msg,
                                    dlq_error = %dlq_err,
                                    "Failed to write to DLQ - event may be lost!"
                                );
                            }
                        } else {
                            tracing::error!(
                                subject = %msg.subject,
                                error = %error_msg,
                                "Failed to extract event_id or tenant_id from envelope for DLQ"
                            );
                        }
                    }
                    Err(parse_err) => {
                        tracing::error!(
                            subject = %msg.subject,
                            error = %error_msg,
                            parse_error = %parse_err,
                            "Failed to process event and could not parse envelope for DLQ"
                        );
                    }
                }
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
