use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Payment method record from ar_payment_methods table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentMethod {
    pub id: i32,
    pub app_id: String,
    pub ar_customer_id: i32,
    pub tilled_payment_method_id: String,
    pub status: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub payment_type: String,
    pub brand: Option<String>,
    pub last4: Option<String>,
    pub exp_month: Option<i32>,
    pub exp_year: Option<i32>,
    pub bank_name: Option<String>,
    pub bank_last4: Option<String>,
    pub is_default: bool,
    pub metadata: Option<JsonValue>,
    pub deleted_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for adding a payment method
#[derive(Debug, Deserialize)]
pub struct AddPaymentMethodRequest {
    pub ar_customer_id: i32,
    pub tilled_payment_method_id: String,
}

/// Request body for updating a payment method
#[derive(Debug, Deserialize)]
pub struct UpdatePaymentMethodRequest {
    pub metadata: Option<JsonValue>,
}

/// Request body for setting default payment method
#[derive(Debug, Deserialize)]
pub struct SetDefaultPaymentMethodRequest {
    pub tilled_payment_method_id: String,
}

/// Query parameters for listing payment methods
#[derive(Debug, Deserialize)]
pub struct ListPaymentMethodsQuery {
    pub customer_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
