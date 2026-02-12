use serde::{Deserialize, Serialize};

// ============================================================================
// INCOMING EVENT PAYLOADS
// ============================================================================

/// Payload for ar.invoice.issued event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceIssuedPayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub amount_due_minor: i32,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
}

/// Payload for payments.payment.succeeded event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentSucceededPayload {
    pub payment_id: String,
    pub invoice_id: String,
    pub ar_customer_id: String,
    pub amount_minor: i32,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processor_payment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_ref: Option<String>,
}

/// Payload for payments.payment.failed event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentFailedPayload {
    pub payment_id: String,
    pub invoice_id: String,
    pub ar_customer_id: String,
    pub amount_minor: i32,
    pub currency: String,
    pub failure_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processor_payment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<i32>,
}

// ============================================================================
// OUTGOING EVENT PAYLOADS
// ============================================================================

/// Payload for notifications.delivery.succeeded event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationDeliverySucceededPayload {
    pub notification_id: String,
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_message_id: Option<String>,
    pub attempts: i32,
}

// ============================================================================
// METADATA TYPES
// ============================================================================

/// Metadata extracted from incoming event envelope
#[derive(Debug, Clone)]
pub struct EnvelopeMetadata {
    pub event_id: uuid::Uuid,
    pub tenant_id: String,
    pub correlation_id: Option<String>,
}
