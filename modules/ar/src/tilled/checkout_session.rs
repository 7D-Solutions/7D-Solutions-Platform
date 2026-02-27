use super::error::TilledError;
use super::types::Metadata;
use super::TilledClient;
use serde::{Deserialize, Serialize};

/// Checkout session response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub payment_intent_id: Option<String>,
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub success_url: Option<String>,
    #[serde(default)]
    pub cancel_url: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub amount_total: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub line_items: Option<Vec<CheckoutLineItem>>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Line item within a checkout session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutLineItem {
    pub quantity: i64,
    pub price_data: CheckoutPriceData,
}

/// Price data for a checkout line item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutPriceData {
    pub currency: String,
    pub unit_amount: i64,
    pub product_data: CheckoutProductData,
}

/// Product data for a checkout line item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutProductData {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Request body for creating a checkout session
#[derive(Debug, Clone, Serialize)]
pub struct CreateCheckoutSessionRequest {
    pub line_items: Vec<CheckoutLineItem>,
    pub payment_intent_data: PaymentIntentData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// Payment intent configuration embedded in checkout session creation
#[derive(Debug, Clone, Serialize)]
pub struct PaymentIntentData {
    pub payment_method_types: Vec<String>,
}

impl TilledClient {
    /// Create a checkout session
    pub async fn create_checkout_session(
        &self,
        request: CreateCheckoutSessionRequest,
    ) -> Result<CheckoutSession, TilledError> {
        if request.line_items.is_empty() {
            return Err(TilledError::ValidationError(
                "line_items must not be empty".to_string(),
            ));
        }
        self.post("/v1/checkout-sessions", &request).await
    }

    /// Get a checkout session by ID
    pub async fn get_checkout_session(&self, id: &str) -> Result<CheckoutSession, TilledError> {
        self.get(&format!("/v1/checkout-sessions/{id}"), None).await
    }

    /// List checkout sessions with optional filters
    pub async fn list_checkout_sessions(
        &self,
        filters: Option<std::collections::HashMap<String, String>>,
    ) -> Result<super::types::ListResponse<CheckoutSession>, TilledError> {
        self.get("/v1/checkout-sessions", filters).await
    }

    /// Expire a checkout session
    pub async fn expire_checkout_session(&self, id: &str) -> Result<CheckoutSession, TilledError> {
        // POST with empty body
        self.post(
            &format!("/v1/checkout-sessions/{id}/expire"),
            &serde_json::json!({}),
        )
        .await
    }
}
