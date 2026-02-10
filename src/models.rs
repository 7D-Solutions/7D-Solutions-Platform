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

/// Subscription status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "billing_subscriptions_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Incomplete,
    IncompleteExpired,
    Trialing,
    Active,
    PastDue,
    Canceled,
    Unpaid,
    Paused,
}

/// Subscription interval enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "billing_subscriptions_interval", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionInterval {
    Day,
    Week,
    Month,
    Year,
}

/// Subscription record from billing_subscriptions table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: i32,
    pub app_id: String,
    pub billing_customer_id: i32,
    pub tilled_subscription_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub price_cents: i32,
    pub status: SubscriptionStatus,
    pub interval_unit: SubscriptionInterval,
    pub interval_count: i32,
    pub billing_cycle_anchor: Option<NaiveDateTime>,
    pub current_period_start: NaiveDateTime,
    pub current_period_end: NaiveDateTime,
    pub cancel_at_period_end: bool,
    pub cancel_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
    pub payment_method_id: String,
    pub payment_method_type: String,
    pub metadata: Option<JsonValue>,
    pub update_source: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating a subscription
#[derive(Debug, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub billing_customer_id: i32,
    pub payment_method_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub price_cents: i32,
    pub interval_unit: Option<SubscriptionInterval>,
    pub interval_count: Option<i32>,
    pub metadata: Option<JsonValue>,
}

/// Request body for updating a subscription
#[derive(Debug, Deserialize)]
pub struct UpdateSubscriptionRequest {
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub price_cents: Option<i32>,
    pub metadata: Option<JsonValue>,
}

/// Request body for canceling a subscription
#[derive(Debug, Deserialize)]
pub struct CancelSubscriptionRequest {
    pub cancel_at_period_end: Option<bool>,
}

/// Query parameters for listing subscriptions
#[derive(Debug, Deserialize)]
pub struct ListSubscriptionsQuery {
    pub customer_id: Option<i32>,
    pub status: Option<SubscriptionStatus>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
