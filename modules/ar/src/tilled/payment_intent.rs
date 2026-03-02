use super::error::TilledError;
use super::types::{normalize_currency, ListResponse, Metadata, PaymentIntent};
use super::TilledClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct CreatePaymentIntentRequest {
    pub amount: i64,
    pub currency: String,
    pub payment_method_types: Vec<String>,
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

#[derive(Debug, Serialize)]
pub struct UpdatePaymentIntentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub struct ChargeResponse {
    /// Payment intent ID (`pi_...`)
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub amount: Option<i64>,
    /// Actual charge ID (`ch_...`) from the charges array, if available.
    pub charge_id: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
}

impl TilledClient {
    /// Create a payment intent
    pub async fn create_payment_intent(
        &self,
        mut request: CreatePaymentIntentRequest,
    ) -> Result<PaymentIntent, TilledError> {
        request.currency = normalize_currency(&request.currency)?;
        if request.payment_method_types.is_empty() {
            request.payment_method_types = vec!["card".to_string()];
        }
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

    /// List payment intents with optional filters
    pub async fn list_payment_intents(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<PaymentIntent>, TilledError> {
        self.get("/v1/payment-intents", filters).await
    }

    /// Get a payment intent by ID
    pub async fn get_payment_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<PaymentIntent, TilledError> {
        self.get(&format!("/v1/payment-intents/{payment_intent_id}"), None)
            .await
    }

    /// Update a payment intent (amount, metadata, etc)
    pub async fn update_payment_intent(
        &self,
        payment_intent_id: &str,
        request: UpdatePaymentIntentRequest,
    ) -> Result<PaymentIntent, TilledError> {
        self.patch(
            &format!("/v1/payment-intents/{payment_intent_id}"),
            &request,
        )
        .await
    }

    /// Cancel a payment intent
    pub async fn cancel_payment_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<PaymentIntent, TilledError> {
        let empty: HashMap<String, String> = HashMap::new();
        self.post(
            &format!("/v1/payment-intents/{payment_intent_id}/cancel"),
            &empty,
        )
        .await
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
        let request = CreatePaymentIntentRequest {
            amount: amount_cents,
            currency: currency.unwrap_or_else(|| "usd".to_string()),
            payment_method_types: vec!["card".to_string()],
            customer_id: Some(customer_id),
            payment_method_id: Some(payment_method_id),
            description,
            metadata,
            confirm: Some(true),
            capture_method: Some("automatic".to_string()),
        };

        let payment_intent = self.create_payment_intent(request).await?;

        let charge_id = payment_intent.charges.first().map(|c| c.id.clone());

        Ok(ChargeResponse {
            id: payment_intent.id,
            status: if payment_intent.status == "succeeded" {
                "succeeded".to_string()
            } else {
                "pending".to_string()
            },
            amount: Some(payment_intent.amount),
            charge_id,
            failure_code: payment_intent
                .last_payment_error
                .as_ref()
                .and_then(|e| e.code.clone()),
            failure_message: payment_intent
                .last_payment_error
                .as_ref()
                .and_then(|e| e.message.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CreatePaymentIntentRequest;

    #[test]
    fn create_payment_intent_payload_includes_required_payment_method_types() {
        let payload = CreatePaymentIntentRequest {
            amount: 1000,
            currency: "usd".to_string(),
            payment_method_types: vec!["card".to_string()],
            customer_id: Some("cus_123".to_string()),
            payment_method_id: Some("pm_123".to_string()),
            description: Some("test".to_string()),
            metadata: None,
            confirm: Some(true),
            capture_method: Some("automatic".to_string()),
        };

        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["payment_method_types"][0], "card");
        assert_eq!(json["currency"], "usd");
    }
}
