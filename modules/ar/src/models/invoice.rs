use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use utoipa::ToSchema;
pub use uuid::Uuid;

/// A single line item on an invoice.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InvoiceLineItem {
    pub description: String,
    pub amount_cents: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
}

/// Invoice status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Invoice {
    pub id: i32,
    pub app_id: String,
    pub tilled_invoice_id: String,
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: String,
    pub amount_cents: i64,
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
    /// Optional link to a Party record in the party-master service.
    #[sqlx(default)]
    pub party_id: Option<Uuid>,
}

/// Request body for creating an invoice
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInvoiceRequest {
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: Option<String>,
    pub amount_cents: i64,
    pub currency: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub metadata: Option<JsonValue>,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
    pub line_item_details: Option<Vec<InvoiceLineItem>>,
    pub compliance_codes: Option<JsonValue>,
    pub correlation_id: Option<String>,
    /// Optional link to a Party record in the party-master service.
    pub party_id: Option<Uuid>,
}

/// Request body for updating an invoice
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateInvoiceRequest {
    pub status: Option<String>,
    pub amount_cents: Option<i64>,
    pub due_at: Option<NaiveDateTime>,
    pub metadata: Option<JsonValue>,
}

/// Request body for finalizing an invoice
#[derive(Debug, Deserialize, ToSchema)]
pub struct FinalizeInvoiceRequest {
    pub paid_at: Option<NaiveDateTime>,
}

/// Query parameters for listing invoices
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListInvoicesQuery {
    pub customer_id: Option<i32>,
    pub subscription_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Invoice attempt status enum (Phase 15 lifecycle states)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, ToSchema)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_invoice_request_party_id_is_optional() {
        let req = CreateInvoiceRequest {
            ar_customer_id: 1,
            subscription_id: None,
            status: None,
            amount_cents: 1000,
            currency: None,
            due_at: None,
            metadata: None,
            billing_period_start: None,
            billing_period_end: None,
            line_item_details: None,
            compliance_codes: None,
            correlation_id: None,
            party_id: None,
        };
        assert!(req.party_id.is_none());
    }

    #[test]
    fn create_invoice_request_accepts_party_id() {
        let id = Uuid::new_v4();
        let req = CreateInvoiceRequest {
            ar_customer_id: 1,
            subscription_id: None,
            status: None,
            amount_cents: 500,
            currency: Some("usd".to_string()),
            due_at: None,
            metadata: None,
            billing_period_start: None,
            billing_period_end: None,
            line_item_details: None,
            compliance_codes: None,
            correlation_id: None,
            party_id: Some(id),
        };
        assert_eq!(req.party_id, Some(id));
    }
}
