use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Subscription status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "ar_subscriptions_status", rename_all = "snake_case")]
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
#[sqlx(type_name = "ar_subscriptions_interval", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionInterval {
    Day,
    Week,
    Month,
    Year,
}

/// Subscription record from ar_subscriptions table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: i32,
    pub app_id: String,
    pub ar_customer_id: i32,
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
    pub ar_customer_id: i32,
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
