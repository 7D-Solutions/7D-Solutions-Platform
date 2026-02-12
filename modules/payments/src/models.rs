use serde::{Deserialize, Serialize};

// ============================================================================
// INCOMING EVENT PAYLOADS
// ============================================================================

/// Payload for ar.payment.collection.requested event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentCollectionRequestedPayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub amount_minor: i32,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_id: Option<String>,
}

// ============================================================================
// OUTGOING EVENT PAYLOADS
// ============================================================================

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
    pub failure_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_ref: Option<String>,
}

// ============================================================================
// PAYMENT PROCESSOR MODELS
// ============================================================================

/// Result from payment processor
#[derive(Debug, Clone)]
pub struct PaymentResult {
    pub payment_id: String,
    pub processor_payment_id: String,
    pub payment_method_ref: Option<String>,
}
