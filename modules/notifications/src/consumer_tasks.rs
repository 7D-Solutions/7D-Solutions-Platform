use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;

use crate::consumer::EventConsumer;
use crate::handlers::{handle_invoice_issued, handle_payment_failed, handle_payment_succeeded};
use crate::models::{
    EnvelopeMetadata, InvoiceIssuedPayload, PaymentFailedPayload, PaymentSucceededPayload,
};

/// Start consumer task for ar.invoice.issued events
pub async fn start_invoice_issued_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting invoice issued consumer");

        // Subscribe to AR invoice issued events
        let subject = "ar.events.ar.invoice.issued";
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
            if let Err(e) = process_invoice_issued(&consumer, &pool, &msg).await {
                tracing::error!(
                    subject = %msg.subject,
                    error = %e,
                    "Failed to process invoice issued event"
                );
            }
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
        let subject = "payments.events.payments.payment.succeeded";
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
            if let Err(e) = process_payment_succeeded(&consumer, &pool, &msg).await {
                tracing::error!(
                    subject = %msg.subject,
                    error = %e,
                    "Failed to process payment succeeded event"
                );
            }
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
        let subject = "payments.events.payments.payment.failed";
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
            if let Err(e) = process_payment_failed(&consumer, &pool, &msg).await {
                tracing::error!(
                    subject = %msg.subject,
                    error = %e,
                    "Failed to process payment failed event"
                );
            }
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
fn extract_metadata(
    envelope: &serde_json::Value,
) -> Result<EnvelopeMetadata, Box<dyn std::error::Error>> {
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

    Ok(EnvelopeMetadata {
        event_id,
        tenant_id,
        correlation_id,
    })
}
