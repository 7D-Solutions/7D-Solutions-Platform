use super::error::TilledError;
use super::types::{Metadata, Subscription};
use super::TilledClient;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CreateSubscriptionRequest {
    pub customer_id: String,
    pub payment_method_id: String,
    pub price: i64,
    pub currency: String,
    pub interval_unit: String,
    pub interval_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_cycle_anchor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_at_period_end: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Serialize)]
pub struct UpdateSubscriptionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_at_period_end: Option<bool>,
}

#[derive(Debug, Default)]
pub struct SubscriptionOptions {
    pub interval_unit: Option<String>,
    pub interval_count: Option<i32>,
    pub billing_cycle_anchor: Option<i64>,
    pub trial_end: Option<i64>,
    pub cancel_at_period_end: Option<bool>,
    pub metadata: Option<Metadata>,
}

impl TilledClient {
    /// Create a subscription
    pub async fn create_subscription(
        &self,
        customer_id: String,
        payment_method_id: String,
        price_cents: i64,
        options: Option<SubscriptionOptions>,
    ) -> Result<Subscription, TilledError> {
        let opts = options.unwrap_or_default();

        let request = CreateSubscriptionRequest {
            customer_id,
            payment_method_id,
            price: price_cents,
            currency: "usd".to_string(),
            interval_unit: opts.interval_unit.unwrap_or_else(|| "month".to_string()),
            interval_count: opts.interval_count.unwrap_or(1),
            billing_cycle_anchor: opts.billing_cycle_anchor,
            trial_end: opts.trial_end,
            cancel_at_period_end: opts.cancel_at_period_end,
            metadata: opts.metadata,
        };

        self.post("/v1/subscriptions", &request).await
    }

    /// Update a subscription
    pub async fn update_subscription(
        &self,
        subscription_id: &str,
        updates: UpdateSubscriptionRequest,
    ) -> Result<Subscription, TilledError> {
        let path = format!("/v1/subscriptions/{}", subscription_id);
        self.patch(&path, &updates).await
    }

    /// Cancel a subscription
    pub async fn cancel_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Subscription, TilledError> {
        let path = format!("/v1/subscriptions/{}/cancel", subscription_id);
        let empty: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        self.post(&path, &empty).await
    }

    /// Get a subscription by ID
    pub async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Subscription, TilledError> {
        let path = format!("/v1/subscriptions/{}", subscription_id);
        self.get(&path, None).await
    }
}
