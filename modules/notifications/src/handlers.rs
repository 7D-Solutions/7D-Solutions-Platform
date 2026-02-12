use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{enqueue_event, EventEnvelope};
use crate::models::{
    EnvelopeMetadata, InvoiceIssuedPayload, NotificationDeliverySucceededPayload,
    PaymentFailedPayload, PaymentSucceededPayload,
};

/// Handle ar.invoice.issued event
///
/// This handler:
/// 1. Receives invoice issued notification from AR
/// 2. Mocks sending a notification (e.g., email)
/// 3. Emits notifications.delivery.succeeded event
pub async fn handle_invoice_issued(
    pool: &PgPool,
    payload: InvoiceIssuedPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        invoice_id = %payload.invoice_id,
        customer_id = %payload.customer_id,
        amount = payload.amount_due_minor,
        "Handling invoice issued notification"
    );

    // Mock: Send invoice notification (email, SMS, etc.)
    let notification_id = Uuid::new_v4().to_string();
    let channel = "email";
    let template_id = "invoice_issued";

    tracing::info!(
        notification_id = %notification_id,
        channel = %channel,
        template = %template_id,
        "Mock: Sending invoice issued notification"
    );

    // Simulate successful delivery
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Construct delivery success event payload
    let success_payload = NotificationDeliverySucceededPayload {
        notification_id: notification_id.clone(),
        channel: channel.to_string(),
        to: Some(format!("customer-{}", payload.customer_id)),
        template_id: Some(template_id.to_string()),
        status: "succeeded".to_string(),
        provider_message_id: Some(format!("mock-msg-{}", Uuid::new_v4())),
        attempts: 1,
    };

    // Create event envelope
    let envelope = EventEnvelope {
        event_id: Uuid::new_v4(),
        occurred_at: chrono::Utc::now(),
        tenant_id: metadata.tenant_id.clone(),
        source_module: "notifications".to_string(),
        source_version: env!("CARGO_PKG_VERSION").to_string(),
        correlation_id: metadata.correlation_id.clone(),
        causation_id: Some(metadata.event_id.to_string()),
        payload: serde_json::to_value(success_payload)?,
    };

    // Enqueue delivery success event to outbox
    let mut tx = pool.begin().await?;
    enqueue_event(&mut tx, "notifications.delivery.succeeded", &envelope).await?;
    tx.commit().await?;

    tracing::info!(
        notification_id = %notification_id,
        invoice_id = %payload.invoice_id,
        event_id = %envelope.event_id,
        "Invoice notification delivery succeeded event enqueued"
    );

    Ok(())
}

/// Handle payments.payment.succeeded event
///
/// This handler:
/// 1. Receives payment success notification from Payments
/// 2. Mocks sending a notification (e.g., email, SMS)
/// 3. Emits notifications.delivery.succeeded event
pub async fn handle_payment_succeeded(
    pool: &PgPool,
    payload: PaymentSucceededPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        payment_id = %payload.payment_id,
        invoice_id = %payload.invoice_id,
        customer_id = %payload.ar_customer_id,
        amount = payload.amount_minor,
        "Handling payment succeeded notification"
    );

    // Mock: Send payment success notification (email, SMS, etc.)
    let notification_id = Uuid::new_v4().to_string();
    let channel = "email";
    let template_id = "payment_succeeded";

    tracing::info!(
        notification_id = %notification_id,
        channel = %channel,
        template = %template_id,
        "Mock: Sending payment succeeded notification"
    );

    // Simulate successful delivery
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Construct delivery success event payload
    let success_payload = NotificationDeliverySucceededPayload {
        notification_id: notification_id.clone(),
        channel: channel.to_string(),
        to: Some(format!("customer-{}", payload.ar_customer_id)),
        template_id: Some(template_id.to_string()),
        status: "succeeded".to_string(),
        provider_message_id: Some(format!("mock-msg-{}", Uuid::new_v4())),
        attempts: 1,
    };

    // Create event envelope
    let envelope = EventEnvelope {
        event_id: Uuid::new_v4(),
        occurred_at: chrono::Utc::now(),
        tenant_id: metadata.tenant_id.clone(),
        source_module: "notifications".to_string(),
        source_version: env!("CARGO_PKG_VERSION").to_string(),
        correlation_id: metadata.correlation_id.clone(),
        causation_id: Some(metadata.event_id.to_string()),
        payload: serde_json::to_value(success_payload)?,
    };

    // Enqueue delivery success event to outbox
    let mut tx = pool.begin().await?;
    enqueue_event(&mut tx, "notifications.delivery.succeeded", &envelope).await?;
    tx.commit().await?;

    tracing::info!(
        notification_id = %notification_id,
        payment_id = %payload.payment_id,
        event_id = %envelope.event_id,
        "Payment succeeded notification delivery event enqueued"
    );

    Ok(())
}

/// Handle payments.payment.failed event
///
/// This handler:
/// 1. Receives payment failure notification from Payments
/// 2. Mocks sending a notification (e.g., email, SMS)
/// 3. Emits notifications.delivery.succeeded event
pub async fn handle_payment_failed(
    pool: &PgPool,
    payload: PaymentFailedPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        payment_id = %payload.payment_id,
        invoice_id = %payload.invoice_id,
        customer_id = %payload.ar_customer_id,
        amount = payload.amount_minor,
        failure_code = %payload.failure_code,
        "Handling payment failed notification"
    );

    // Mock: Send payment failure notification (email, SMS, etc.)
    let notification_id = Uuid::new_v4().to_string();
    let channel = "email";
    let template_id = "payment_failed";

    tracing::info!(
        notification_id = %notification_id,
        channel = %channel,
        template = %template_id,
        "Mock: Sending payment failed notification"
    );

    // Simulate successful delivery
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Construct delivery success event payload
    let success_payload = NotificationDeliverySucceededPayload {
        notification_id: notification_id.clone(),
        channel: channel.to_string(),
        to: Some(format!("customer-{}", payload.ar_customer_id)),
        template_id: Some(template_id.to_string()),
        status: "succeeded".to_string(),
        provider_message_id: Some(format!("mock-msg-{}", Uuid::new_v4())),
        attempts: 1,
    };

    // Create event envelope
    let envelope = EventEnvelope {
        event_id: Uuid::new_v4(),
        occurred_at: chrono::Utc::now(),
        tenant_id: metadata.tenant_id.clone(),
        source_module: "notifications".to_string(),
        source_version: env!("CARGO_PKG_VERSION").to_string(),
        correlation_id: metadata.correlation_id.clone(),
        causation_id: Some(metadata.event_id.to_string()),
        payload: serde_json::to_value(success_payload)?,
    };

    // Enqueue delivery success event to outbox
    let mut tx = pool.begin().await?;
    enqueue_event(&mut tx, "notifications.delivery.succeeded", &envelope).await?;
    tx.commit().await?;

    tracing::info!(
        notification_id = %notification_id,
        payment_id = %payload.payment_id,
        event_id = %envelope.event_id,
        "Payment failed notification delivery event enqueued"
    );

    Ok(())
}
