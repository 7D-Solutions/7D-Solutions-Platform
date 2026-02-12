use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::consumer::EventConsumer;
use crate::envelope_validation::validate_envelope;
use crate::handlers::{handle_invoice_issued, handle_payment_failed, handle_payment_succeeded};
use crate::models::{
    EnvelopeMetadata, InvoiceIssuedPayload, PaymentFailedPayload, PaymentSucceededPayload,
};

/// Start consumer task for ar.invoice.issued events
pub async fn start_invoice_issued_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting invoice issued consumer");

        // Subscribe to AR invoice issued events
        let subject = "ar.events.invoice.issued";
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
            // Extract correlation fields from envelope for observability
            let (event_id, tenant_id, correlation_id, source_module) =
                match extract_correlation_fields(&msg) {
                    Ok(fields) => fields,
                    Err(e) => {
                        tracing::error!(
                            subject = %msg.subject,
                            error = %e,
                            "Failed to extract correlation fields from envelope"
                        );
                        continue;
                    }
                };

            // Create tracing span with correlation fields
            let span = tracing::info_span!(
                "process_event",
                event_id = %event_id,
                subject = %msg.subject,
                tenant_id = %tenant_id,
                correlation_id = %correlation_id.as_deref().unwrap_or("none"),
                source_module = %source_module.as_deref().unwrap_or("unknown")
            );

            // Process message within the span
            async {
                let consumer_clone = consumer.clone();
                let pool_clone = pool.clone();
                let msg_clone = msg.clone();

                let result = retry_with_backoff(
                    || {
                        let consumer = consumer_clone.clone();
                        let pool = pool_clone.clone();
                        let msg = msg_clone.clone();
                        async move {
                            process_invoice_issued(&consumer, &pool, &msg)
                                .await
                                .map_err(|e| format!("{:#}", e))
                        }
                    },
                    &retry_config,
                    "notifications_invoice_issued_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        retry_count = retry_config.max_attempts,
                        "Event processing failed after retries, sending to DLQ"
                    );

                    crate::dlq::handle_processing_error(
                        &pool,
                        &msg,
                        &error_msg,
                        retry_config.max_attempts as i32,
                    )
                    .await;
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("Invoice issued consumer stopped");
    });
}

async fn process_invoice_issued(
    consumer: &EventConsumer,
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    consumer
        .process_idempotent(msg, |payload: InvoiceIssuedPayload| async {
            // Extract envelope metadata
            let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
            let metadata = extract_metadata(&envelope)?;

            // Handle the invoice issued event
            handle_invoice_issued(pool, payload, metadata).await
        })
        .await
}

/// Start consumer task for payments.payment.succeeded events
pub async fn start_payment_succeeded_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting payment succeeded consumer");

        // Subscribe to Payments payment succeeded events
        let subject = "payments.events.payment.succeeded";
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
            // Extract correlation fields from envelope for observability
            let (event_id, tenant_id, correlation_id, source_module) =
                match extract_correlation_fields(&msg) {
                    Ok(fields) => fields,
                    Err(e) => {
                        tracing::error!(
                            subject = %msg.subject,
                            error = %e,
                            "Failed to extract correlation fields from envelope"
                        );
                        continue;
                    }
                };

            // Create tracing span with correlation fields
            let span = tracing::info_span!(
                "process_event",
                event_id = %event_id,
                subject = %msg.subject,
                tenant_id = %tenant_id,
                correlation_id = %correlation_id.as_deref().unwrap_or("none"),
                source_module = %source_module.as_deref().unwrap_or("unknown")
            );

            // Process message within the span
            async {
                let consumer_clone = consumer.clone();
                let pool_clone = pool.clone();
                let msg_clone = msg.clone();

                let result = retry_with_backoff(
                    || {
                        let consumer = consumer_clone.clone();
                        let pool = pool_clone.clone();
                        let msg = msg_clone.clone();
                        async move {
                            process_payment_succeeded(&consumer, &pool, &msg)
                                .await
                                .map_err(|e| format!("{:#}", e))
                        }
                    },
                    &retry_config,
                    "notifications_payment_succeeded_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        retry_count = retry_config.max_attempts,
                        "Event processing failed after retries, sending to DLQ"
                    );

                    crate::dlq::handle_processing_error(
                        &pool,
                        &msg,
                        &error_msg,
                        retry_config.max_attempts as i32,
                    )
                    .await;
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("Payment succeeded consumer stopped");
    });
}

async fn process_payment_succeeded(
    consumer: &EventConsumer,
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    consumer
        .process_idempotent(msg, |payload: PaymentSucceededPayload| async {
            // Extract envelope metadata
            let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
            let metadata = extract_metadata(&envelope)?;

            // Handle the payment succeeded event
            handle_payment_succeeded(pool, payload, metadata).await
        })
        .await
}

/// Start consumer task for payments.payment.failed events
pub async fn start_payment_failed_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting payment failed consumer");

        // Subscribe to Payments payment failed events
        let subject = "payments.events.payment.failed";
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
            // Extract correlation fields from envelope for observability
            let (event_id, tenant_id, correlation_id, source_module) =
                match extract_correlation_fields(&msg) {
                    Ok(fields) => fields,
                    Err(e) => {
                        tracing::error!(
                            subject = %msg.subject,
                            error = %e,
                            "Failed to extract correlation fields from envelope"
                        );
                        continue;
                    }
                };

            // Create tracing span with correlation fields
            let span = tracing::info_span!(
                "process_event",
                event_id = %event_id,
                subject = %msg.subject,
                tenant_id = %tenant_id,
                correlation_id = %correlation_id.as_deref().unwrap_or("none"),
                source_module = %source_module.as_deref().unwrap_or("unknown")
            );

            // Process message within the span
            async {
                let consumer_clone = consumer.clone();
                let pool_clone = pool.clone();
                let msg_clone = msg.clone();

                let result = retry_with_backoff(
                    || {
                        let consumer = consumer_clone.clone();
                        let pool = pool_clone.clone();
                        let msg = msg_clone.clone();
                        async move {
                            process_payment_failed(&consumer, &pool, &msg)
                                .await
                                .map_err(|e| format!("{:#}", e))
                        }
                    },
                    &retry_config,
                    "notifications_payment_failed_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        retry_count = retry_config.max_attempts,
                        "Event processing failed after retries, sending to DLQ"
                    );

                    crate::dlq::handle_processing_error(
                        &pool,
                        &msg,
                        &error_msg,
                        retry_config.max_attempts as i32,
                    )
                    .await;
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("Payment failed consumer stopped");
    });
}

async fn process_payment_failed(
    consumer: &EventConsumer,
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    consumer
        .process_idempotent(msg, |payload: PaymentFailedPayload| async {
            // Extract envelope metadata
            let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
            let metadata = extract_metadata(&envelope)?;

            // Handle the payment failed event
            handle_payment_failed(pool, payload, metadata).await
        })
        .await
}

/// Extract envelope metadata from event payload
///
/// First validates the envelope fields according to platform event contract,
/// then extracts metadata for event processing.
fn extract_metadata(
    envelope: &serde_json::Value,
) -> Result<EnvelopeMetadata, Box<dyn std::error::Error>> {
    // Validate envelope fields first
    validate_envelope(envelope).map_err(|e| format!("Envelope validation failed: {}", e))?;

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

    // Accept both "correlation_id" (Payments envelope) and "trace_id" (AR envelope)
    let correlation_id = envelope
        .get("correlation_id")
        .or_else(|| envelope.get("trace_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(EnvelopeMetadata {
        event_id,
        tenant_id,
        correlation_id,
    })
}

/// Extract correlation fields from event envelope for observability
///
/// Returns: (event_id, tenant_id, correlation_id, source_module)
fn extract_correlation_fields(
    msg: &BusMessage,
) -> Result<(Uuid, String, Option<String>, Option<String>), Box<dyn std::error::Error>> {
    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;

    let event_id_str = envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing event_id")?;
    let event_id = Uuid::parse_str(event_id_str)?;

    let tenant_id = envelope
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing tenant_id")?
        .to_string();

    let correlation_id = envelope
        .get("correlation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let source_module = envelope
        .get("source_module")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((event_id, tenant_id, correlation_id, source_module))
}
