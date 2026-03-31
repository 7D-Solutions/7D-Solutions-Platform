use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use utoipa::ToSchema;

/// Refund record from ar_refunds table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Refund {
    pub id: i32,
    pub app_id: String,
    pub ar_customer_id: i32,
    pub charge_id: i32,
    pub tilled_refund_id: Option<String>,
    pub tilled_charge_id: Option<String>,
    pub status: String,
    pub amount_cents: i32,
    pub currency: String,
    pub reason: Option<String>,
    pub reference_id: String,
    pub note: Option<String>,
    pub metadata: Option<JsonValue>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating a refund
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRefundRequest {
    pub charge_id: i32,
    pub amount_cents: i32,
    pub currency: Option<String>,
    pub reason: Option<String>,
    pub reference_id: String,
    pub note: Option<String>,
    pub metadata: Option<JsonValue>,
}

/// Query parameters for listing refunds
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListRefundsQuery {
    pub charge_id: Option<i32>,
    pub customer_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
