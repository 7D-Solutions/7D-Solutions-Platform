use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{enqueue_event, EventEnvelope};
use crate::models::{PaymentCollectionRequestedPayload, PaymentSucceededPayload};
use crate::processor::MockPaymentProcessor;

/// Handle ar.payment.collection.requested event
///
/// This handler:
/// 1. Receives payment collection request from AR
/// 2. Processes payment using mock processor
/// 3. Emits payments.payment.succeeded event
pub async fn handle_payment_collection_requested(
    pool: &PgPool,
    payload: PaymentCollectionRequestedPayload,
    envelope_metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        invoice_id = %payload.invoice_id,
        customer_id = %payload.customer_id,
        amount = payload.amount_minor,
        "Handling payment collection request"
    );

    // Process payment using mock processor
    let processor = MockPaymentProcessor::new();
    let result = processor.process_payment(&payload).await?;

    // Construct success event payload
    let success_payload = PaymentSucceededPayload {
        payment_id: result.payment_id.clone(),
        invoice_id: payload.invoice_id.clone(),
        ar_customer_id: payload.customer_id.clone(),
        amount_minor: payload.amount_minor,
        currency: payload.currency.clone(),
        processor_payment_id: Some(result.processor_payment_id.clone()),
        payment_method_ref: result.payment_method_ref,
    };

    // Create event envelope
    let envelope = EventEnvelope::new(
        Uuid::new_v4(),
        envelope_metadata.tenant_id,
        envelope_metadata.correlation_id,
        Some(envelope_metadata.event_id.to_string()),
        success_payload,
    );

    // Enqueue success event to outbox
    enqueue_event(pool, "payment.succeeded", &envelope).await?;

    tracing::info!(
        payment_id = %result.payment_id,
        invoice_id = %payload.invoice_id,
        event_id = %envelope.event_id,
        "Payment succeeded event enqueued"
    );

    Ok(())
}

/// Metadata extracted from incoming event envelope
pub struct EnvelopeMetadata {
    pub event_id: Uuid,
    pub tenant_id: String,
    pub correlation_id: Option<String>,
}
