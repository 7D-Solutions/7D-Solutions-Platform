use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Payload for ar.payment.collection.requested event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentCollectionRequestedPayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub amount_minor: i64,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_id: Option<String>,
}

/// Payload for payments.payment.succeeded event (consumed from Payments module)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentSucceededPayload {
    pub payment_id: String,
    pub invoice_id: String,
    pub ar_customer_id: String,
    pub amount_minor: i64,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processor_payment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_ref: Option<String>,
}

/// Payload for gl.posting.requested event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlPostingRequestPayload {
    pub posting_date: String,
    pub currency: String,
    pub source_doc_type: String,
    pub source_doc_id: String,
    pub description: String,
    pub lines: Vec<GlPostingLine>,
}

/// Individual line in a GL posting request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlPostingLine {
    pub account_ref: String,
    pub debit: i64,
    pub credit: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

/// Request body for POST /api/ar/usage — capture metered usage
#[derive(Debug, Deserialize, ToSchema)]
pub struct CaptureUsageRequest {
    pub idempotency_key: Uuid,
    pub customer_id: String,
    pub metric_name: String,
    pub quantity: f64,
    pub unit: String,
    pub unit_price_minor: i64,
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
    pub subscription_id: Option<i32>,
}

/// Response for a captured usage record
#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct UsageRecord {
    pub id: i32,
    pub usage_uuid: Uuid,
    pub idempotency_key: Option<Uuid>,
    pub app_id: String,
    pub customer_id: i32,
    pub metric_name: String,
    pub quantity: f64,
    pub unit: String,
    pub unit_price_cents: i32,
    pub period_start: NaiveDateTime,
    pub period_end: NaiveDateTime,
    pub recorded_at: NaiveDateTime,
}
