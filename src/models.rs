use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Customer record from billing_customers table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Customer {
    pub id: i32,
    pub app_id: String,
    pub external_customer_id: Option<String>,
    pub tilled_customer_id: Option<String>,
    pub status: String,
    pub email: String,
    pub name: Option<String>,
    pub default_payment_method_id: Option<String>,
    pub payment_method_type: Option<String>,
    pub metadata: Option<JsonValue>,
    pub update_source: Option<String>,
    pub updated_by: Option<String>,
    pub delinquent_since: Option<NaiveDateTime>,
    pub grace_period_end: Option<NaiveDateTime>,
    pub next_retry_at: Option<NaiveDateTime>,
    pub retry_attempt_count: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating a customer
#[derive(Debug, Deserialize)]
pub struct CreateCustomerRequest {
    pub email: String,
    pub name: Option<String>,
    pub external_customer_id: Option<String>,
    pub metadata: Option<JsonValue>,
}

/// Request body for updating a customer
#[derive(Debug, Deserialize)]
pub struct UpdateCustomerRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub metadata: Option<JsonValue>,
}

/// Query parameters for listing customers
#[derive(Debug, Deserialize)]
pub struct ListCustomersQuery {
    pub external_customer_id: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}
