use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// INCOMING EVENT PAYLOADS
// ============================================================================

/// Payload for inventory.low_stock_triggered event.
/// Mirrors `inventory_rs::events::low_stock_triggered::LowStockTriggeredPayload`
/// (duplicated here to avoid a cross-crate dependency on the inventory library).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowStockTriggeredPayload {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub location_id: Option<Uuid>,
    pub reorder_point: i64,
    pub available_qty: i64,
    pub triggered_at: DateTime<Utc>,
}

/// Payload for ar.invoice_opened event (from AR module's InvoiceLifecyclePayload)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceIssuedPayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub amount_cents: i32,
    pub currency: String,
    #[serde(default)]
    pub due_at: Option<chrono::NaiveDateTime>,
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
