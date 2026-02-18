use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{enqueue_event, EventEnvelope};
use crate::models::{
    EnvelopeMetadata, InvoiceIssuedPayload, LowStockTriggeredPayload,
    NotificationDeliverySucceededPayload, PaymentFailedPayload, PaymentSucceededPayload,
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
    let envelope = crate::event_bus::create_notifications_envelope(
        Uuid::new_v4(),
        metadata.tenant_id.clone(),
        "notifications.delivery.succeeded".to_string(),
        metadata.correlation_id.clone(),
        Some(metadata.event_id.to_string()),
        "SIDE_EFFECT".to_string(), // Phase 16: Email/SMS delivery is a non-idempotent side effect
        serde_json::to_value(success_payload)?,
    );

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
    let envelope = crate::event_bus::create_notifications_envelope(
        Uuid::new_v4(),
        metadata.tenant_id.clone(),
        "notifications.delivery.succeeded".to_string(),
        metadata.correlation_id.clone(),
        Some(metadata.event_id.to_string()),
        "SIDE_EFFECT".to_string(), // Phase 16: Email/SMS delivery is a non-idempotent side effect
        serde_json::to_value(success_payload)?,
    );

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
    let envelope = crate::event_bus::create_notifications_envelope(
        Uuid::new_v4(),
        metadata.tenant_id.clone(),
        "notifications.delivery.succeeded".to_string(),
        metadata.correlation_id.clone(),
        Some(metadata.event_id.to_string()),
        "SIDE_EFFECT".to_string(), // Phase 16: Email/SMS delivery is a non-idempotent side effect
        serde_json::to_value(success_payload)?,
    );

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

/// Handle inventory.low_stock_triggered event.
///
/// When inventory stock crosses below the reorder point, this handler:
/// 1. Logs the low-stock signal for the item/warehouse.
/// 2. Enqueues a `notifications.low_stock.alert.created` outbox event so the
///    downstream channel (email / webhook) can pick it up.
pub async fn handle_low_stock_triggered(
    pool: &PgPool,
    payload: LowStockTriggeredPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        item_id = %payload.item_id,
        warehouse_id = %payload.warehouse_id,
        available_qty = payload.available_qty,
        reorder_point = payload.reorder_point,
        tenant_id = %payload.tenant_id,
        "Handling low-stock triggered notification"
    );

    let notification_id = Uuid::new_v4().to_string();
    let channel = "internal";
    let template_id = "low_stock_alert";

    let success_payload = NotificationDeliverySucceededPayload {
        notification_id: notification_id.clone(),
        channel: channel.to_string(),
        to: None,
        template_id: Some(template_id.to_string()),
        status: "queued".to_string(),
        provider_message_id: None,
        attempts: 1,
    };

    let envelope = crate::event_bus::create_notifications_envelope(
        Uuid::new_v4(),
        metadata.tenant_id.clone(),
        "notifications.low_stock.alert.created".to_string(),
        metadata.correlation_id.clone(),
        Some(metadata.event_id.to_string()),
        "SIDE_EFFECT".to_string(),
        serde_json::to_value(success_payload)?,
    );

    let mut tx = pool.begin().await?;
    enqueue_event(&mut tx, "notifications.low_stock.alert.created", &envelope).await?;
    tx.commit().await?;

    tracing::info!(
        notification_id = %notification_id,
        item_id = %payload.item_id,
        event_id = %envelope.event_id,
        "Low-stock alert notification enqueued"
    );

    Ok(())
}
