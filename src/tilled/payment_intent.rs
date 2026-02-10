use super::error::TilledError;
use super::types::{Metadata, PaymentIntent};
use super::TilledClient;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct CreatePaymentIntentRequest {
    pub amount: i64,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_method: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfirmPaymentIntentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChargeResponse {
    pub id: String,
    pub status: String,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
}

impl TilledClient {
    /// Create a payment intent
    pub async fn create_payment_intent(
        &self,
        amount: i64,
        currency: String,
        customer_id: Option<String>,
        payment_method_id: Option<String>,
        description: Option<String>,
        metadata: Option<Metadata>,
        confirm: bool,
    ) -> Result<PaymentIntent, TilledError> {
        let request = CreatePaymentIntentRequest {
            amount,
            currency,
            customer_id,
            payment_method_id,
            description,
            metadata,
            confirm: Some(confirm),
            capture_method: Some("automatic".to_string()),
        };

        self.post("/v1/payment-intents", &request).await
    }

    /// Confirm a payment intent
    pub async fn confirm_payment_intent(
        &self,
        payment_intent_id: &str,
        payment_method_id: Option<String>,
    ) -> Result<PaymentIntent, TilledError> {
        let path = format!("/v1/payment-intents/{}/confirm", payment_intent_id);
        let request = ConfirmPaymentIntentRequest { payment_method_id };

        self.post(&path, &request).await
    }

    /// Capture a payment intent
    pub async fn capture_payment_intent(
        &self,
        payment_intent_id: &str,
        amount: Option<i64>,
    ) -> Result<PaymentIntent, TilledError> {
        let path = format!("/v1/payment-intents/{}/capture", payment_intent_id);

        let mut body = serde_json::Map::new();
        if let Some(amt) = amount {
            body.insert("amount".to_string(), serde_json::Value::from(amt));
        }

        self.post(&path, &body).await
    }

    /// Create a one-time charge (convenience method)
    pub async fn create_charge(
        &self,
        customer_id: String,
        payment_method_id: String,
        amount_cents: i64,
        currency: Option<String>,
        description: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<ChargeResponse, TilledError> {
        let payment_intent = self.create_payment_intent(
            amount_cents,
            currency.unwrap_or_else(|| "usd".to_string()),
            Some(customer_id),
            Some(payment_method_id),
            description,
            metadata,
            true, // Auto-confirm
        ).await?;

        Ok(ChargeResponse {
            id: payment_intent.id,
            status: if payment_intent.status == "succeeded" {
                "succeeded".to_string()
            } else {
                "pending".to_string()
            },
            failure_code: payment_intent.last_payment_error.as_ref().map(|e| e.code.clone()),
            failure_message: payment_intent.last_payment_error.as_ref().map(|e| e.message.clone()),
        })
    }
}
