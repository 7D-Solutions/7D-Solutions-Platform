use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::events::{is_event_processed, mark_event_processed};
use crate::models::PaymentSucceededPayload;

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
            // All logs within this span will automatically include these fields
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
                // Clone necessary data for retry closure
                let pool_clone = pool.clone();
                let msg_clone = msg.clone();

                // Retry processing with exponential backoff
                // Wrap in a Send-safe error type (String)
                let result = retry_with_backoff(
                    || {
                        let pool = pool_clone.clone();
                        let msg = msg_clone.clone();
                        async move {
                            process_payment_succeeded(&pool, &msg)
                                .await
                                .map_err(|e| format!("{:#}", e))
                        }
                    },
                    &retry_config,
                    "ar_payment_succeeded_consumer",
                )
                .await;

                // If all retries failed, send to DLQ
                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        retry_count = retry_config.max_attempts,
                        "Event processing failed after retries, sending to DLQ"
                    );

                    crate::events::dlq::handle_processing_error(
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
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse the event envelope
    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;

    // Extract event metadata
    let event_id = envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing event_id")?;
    let event_id = Uuid::parse_str(event_id)?;

    // Check if already processed (idempotency)
    if is_event_processed(pool, event_id).await? {
        tracing::info!(
            event_id = %event_id,
            "Duplicate payment.succeeded event ignored (already processed)"
        );
        return Ok(());
    }

    // Extract the payload
    let payload: PaymentSucceededPayload = serde_json::from_value(
        envelope
            .get("payload")
            .ok_or("Missing payload")?
            .clone(),
    )?;

    tracing::info!(
        event_id = %event_id,
        invoice_id = %payload.invoice_id,
        payment_id = %payload.payment_id,
        amount = %payload.amount_minor,
        "Processing payment.succeeded event"
    );

    // Apply payment to invoice and mark as paid
    handle_payment_succeeded(pool, &payload).await?;

    // Mark event as processed
    mark_event_processed(pool, event_id, "payments.payment.succeeded", "ar-payment-consumer").await?;

    tracing::info!(
        event_id = %event_id,
        invoice_id = %payload.invoice_id,
        "Payment successfully applied to invoice"
    );

    Ok(())
}

/// Handle payment.succeeded event by marking the invoice as paid
async fn handle_payment_succeeded(
    pool: &PgPool,
    payload: &PaymentSucceededPayload,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse invoice_id from String to i32
    let invoice_id: i32 = payload.invoice_id.parse()
        .map_err(|e| format!("Failed to parse invoice_id '{}': {}", payload.invoice_id, e))?;

    // Update invoice status to 'paid'
    let result = sqlx::query(
        r#"
        UPDATE ar_invoices
        SET status = 'paid',
            updated_at = NOW()
        WHERE id = $1
          AND status != 'paid'
        "#,
    )
    .bind(invoice_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        tracing::warn!(
            invoice_id = %payload.invoice_id,
            "Invoice not found or already paid"
        );
    } else {
        tracing::info!(
            invoice_id = %payload.invoice_id,
            payment_id = %payload.payment_id,
            "Invoice marked as paid"
        );
    }

    Ok(())
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
