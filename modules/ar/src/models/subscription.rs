use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use utoipa::ToSchema;
use uuid::Uuid;

/// Subscription status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, ToSchema)]
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
    PendingSync,
    Canceling,
}

/// Subscription interval enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "ar_subscriptions_interval", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionInterval {
    Day,
    Week,
    Month,
    Year,
}

/// Subscription record from ar_subscriptions table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Subscription {
    pub id: i32,
    pub app_id: String,
    pub ar_customer_id: i32,
    pub tilled_subscription_id: Option<String>,
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
    /// Optional link to a Party record in the party-master service.
    #[sqlx(default)]
    pub party_id: Option<Uuid>,
}

/// Request body for creating a subscription
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSubscriptionRequest {
    pub ar_customer_id: i32,
    pub payment_method_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub price_cents: i32,
    pub interval_unit: Option<SubscriptionInterval>,
    pub interval_count: Option<i32>,
    pub metadata: Option<JsonValue>,
    /// Optional link to a Party record in the party-master service.
    pub party_id: Option<Uuid>,
}

/// Request body for updating a subscription
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSubscriptionRequest {
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub price_cents: Option<i32>,
    pub metadata: Option<JsonValue>,
}

/// Request body for canceling a subscription
#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelSubscriptionRequest {
    pub cancel_at_period_end: Option<bool>,
}

/// Query parameters for listing subscriptions
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListSubscriptionsQuery {
    pub customer_id: Option<i32>,
    pub status: Option<SubscriptionStatus>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_subscription_request_party_id_is_optional() {
        let req = CreateSubscriptionRequest {
            ar_customer_id: 1,
            payment_method_id: "pm_123".to_string(),
            plan_id: "plan_abc".to_string(),
            plan_name: "Pro Plan".to_string(),
            price_cents: 999,
            interval_unit: None,
            interval_count: None,
            metadata: None,
            party_id: None,
        };
        assert!(req.party_id.is_none());
    }

    #[test]
    fn create_subscription_request_accepts_party_id() {
        let id = Uuid::new_v4();
        let req = CreateSubscriptionRequest {
            ar_customer_id: 2,
            payment_method_id: "pm_456".to_string(),
            plan_id: "plan_xyz".to_string(),
            plan_name: "Enterprise".to_string(),
            price_cents: 4999,
            interval_unit: None,
            interval_count: None,
            metadata: None,
            party_id: Some(id),
        };
        assert_eq!(req.party_id, Some(id));
    }
}
