use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::enqueue_event;
use crate::models::{
    EnvelopeMetadata, InvoiceIssuedPayload, LowStockTriggeredPayload,
    NotificationDeliverySucceededPayload, PaymentFailedPayload, PaymentSucceededPayload,
};

/// Handle ar.invoice_opened event
///
/// Schedules an `invoice_due_soon` reminder 3 days before the invoice due date.
/// Replay safety is enforced upstream by the `processed_events` idempotency gate.
pub async fn handle_invoice_issued(
    pool: &PgPool,
    payload: InvoiceIssuedPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        invoice_id = %payload.invoice_id,
        customer_id = %payload.customer_id,
        amount = payload.amount_cents,
        "Handling invoice opened notification"
    );

    let due_at_ndt = match payload.due_at {
        Some(dt) => dt,
        None => {
            tracing::debug!(
                invoice_id = %payload.invoice_id,
                "No due_at in payload, skipping reminder scheduling"
            );
            return Ok(());
        }
    };

    let due_at = due_at_ndt.and_utc();
    let deliver_at = due_at - chrono::Duration::days(3);
    let due_date_str = due_at_ndt.date().format("%Y-%m-%d").to_string();

    let recipient_ref = format!("{}:{}", metadata.tenant_id, payload.customer_id);
    let payload_json = serde_json::json!({
        "invoice_id": payload.invoice_id,
        "amount": payload.amount_cents,
        "due_date": due_date_str,
    });

    let id = crate::scheduled::insert_pending(
        pool,
        &recipient_ref,
        "email",
        "invoice_due_soon",
        payload_json,
        deliver_at,
    )
    .await?;

    tracing::info!(
        scheduled_notification_id = %id,
        invoice_id = %payload.invoice_id,
        deliver_at = %deliver_at,
        "Scheduled invoice_due_soon reminder"
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

    let notification_id = Uuid::new_v4().to_string();
    let channel = "email";
    let template_id = "payment_succeeded";

    tracing::info!(
        notification_id = %notification_id,
        channel = %channel,
        template = %template_id,
        "Queuing payment succeeded notification for delivery"
    );

    // Construct delivery success event payload
    let success_payload = NotificationDeliverySucceededPayload {
        notification_id: notification_id.clone(),
        channel: channel.to_string(),
        to: Some(format!("customer-{}", payload.ar_customer_id)),
        template_id: Some(template_id.to_string()),
        status: "succeeded".to_string(),
        provider_message_id: Some(format!("ntf-{}", Uuid::new_v4())),
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
/// Schedules a `payment_retry` reminder 24 hours from now.
/// Replay safety is enforced upstream by the `processed_events` idempotency gate.
pub async fn handle_payment_failed(
    pool: &PgPool,
    payload: PaymentFailedPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        payment_id = %payload.payment_id,
        invoice_id = %payload.invoice_id,
        customer_id = %payload.ar_customer_id,
        failure_code = %payload.failure_code,
        "Handling payment failed notification"
    );

    let deliver_at = Utc::now() + chrono::Duration::hours(24);
    let recipient_ref = format!("{}:{}", metadata.tenant_id, payload.ar_customer_id);
    let payload_json = serde_json::json!({
        "payment_id": payload.payment_id,
        "invoice_id": payload.invoice_id,
        "failure_code": payload.failure_code,
    });

    let id = crate::scheduled::insert_pending(
        pool,
        &recipient_ref,
        "email",
        "payment_retry",
        payload_json,
        deliver_at,
    )
    .await?;

    tracing::info!(
        scheduled_notification_id = %id,
        payment_id = %payload.payment_id,
        deliver_at = %deliver_at,
        "Scheduled payment_retry reminder"
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
