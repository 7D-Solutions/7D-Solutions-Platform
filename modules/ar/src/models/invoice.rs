use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

/// Invoice status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
#[serde(rename_all = "lowercase")]
pub struct InvoiceStatus(pub String);

impl InvoiceStatus {
    pub const DRAFT: &'static str = "draft";
    pub const OPEN: &'static str = "open";
    pub const PAID: &'static str = "paid";
    pub const VOID: &'static str = "void";
    pub const UNCOLLECTIBLE: &'static str = "uncollectible";
}

/// Invoice record from ar_invoices table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Invoice {
    pub id: i32,
    pub app_id: String,
    pub tilled_invoice_id: String,
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: String,
    pub amount_cents: i32,
    pub currency: String,
    pub due_at: Option<NaiveDateTime>,
    pub paid_at: Option<NaiveDateTime>,
    pub hosted_url: Option<String>,
    pub metadata: Option<JsonValue>,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
    pub line_item_details: Option<JsonValue>,
    pub compliance_codes: Option<JsonValue>,
    pub correlation_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating an invoice
#[derive(Debug, Deserialize)]
pub struct CreateInvoiceRequest {
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: Option<String>,
    pub amount_cents: i32,
    pub currency: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub metadata: Option<JsonValue>,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
    pub line_item_details: Option<JsonValue>,
    pub compliance_codes: Option<JsonValue>,
    pub correlation_id: Option<String>,
}

/// Request body for updating an invoice
#[derive(Debug, Deserialize)]
pub struct UpdateInvoiceRequest {
    pub status: Option<String>,
    pub amount_cents: Option<i32>,
    pub due_at: Option<NaiveDateTime>,
    pub metadata: Option<JsonValue>,
}

/// Request body for finalizing an invoice
#[derive(Debug, Deserialize)]
pub struct FinalizeInvoiceRequest {
    pub paid_at: Option<NaiveDateTime>,
}

/// Query parameters for listing invoices
#[derive(Debug, Deserialize)]
pub struct ListInvoicesQuery {
    pub customer_id: Option<i32>,
    pub subscription_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Invoice attempt status enum (Phase 15 lifecycle states)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "ar_invoice_attempt_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum InvoiceAttemptStatus {
    Attempting,
    Succeeded,
    FailedRetry,
    FailedFinal,
}

/// Invoice attempt record from ar_invoice_attempts table
/// Phase 15: Deterministic attempt ledger for exactly-once side effect enforcement
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InvoiceAttempt {
    pub id: Uuid,
    pub app_id: String,
    pub invoice_id: i32,
    pub attempt_no: i32,
    pub status: InvoiceAttemptStatus,
    pub attempted_at: NaiveDateTime,
    pub completed_at: Option<NaiveDateTime>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
