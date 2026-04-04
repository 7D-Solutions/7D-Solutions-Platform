use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// INCOMING EVENT PAYLOADS
// ============================================================================

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

// ============================================================================
// OUTGOING EVENT PAYLOADS
// ============================================================================

/// Payload for payments.payment.succeeded event
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

/// Payload for payments.payment.unknown event
///
/// Emitted when a webhook provides an ambiguous result (PSP timeout, network error, etc.).
/// Requires reconciliation to resolve to a terminal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentUnknownPayload {
    pub payment_id: String,
    pub invoice_id: String,
    pub ar_customer_id: String,
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
    pub amount_minor: i64,
    pub currency: String,
    pub failure_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processor_payment_id: Option<String>,
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

// ============================================================================
// PAYMENT ATTEMPT MODELS (Phase 15)
// ============================================================================

/// Payment attempt status enum (Phase 15 lifecycle states + UNKNOWN protocol)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "payment_attempt_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PaymentAttemptStatus {
    Attempting,
    Succeeded,
    FailedRetry,
    FailedFinal,
    Unknown, // Phase 15 UNKNOWN protocol: blocks retries and subscription suspension
}

/// Payment attempt record from payment_attempts table
/// Phase 15: Deterministic attempt ledger for exactly-once side effect enforcement
/// Supports UNKNOWN protocol for webhook reconciliation
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentAttempt {
    pub id: Uuid,
    pub app_id: String,
    pub payment_id: Uuid,
    pub invoice_id: String,
    pub attempt_no: i32,
    pub status: PaymentAttemptStatus,
    pub attempted_at: NaiveDateTime,
    pub completed_at: Option<NaiveDateTime>,
    pub processor_payment_id: Option<String>,
    pub payment_method_ref: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub webhook_event_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
