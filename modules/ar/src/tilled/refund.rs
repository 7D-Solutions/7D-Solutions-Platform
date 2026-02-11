use super::error::TilledError;
use super::types::{ListResponse, Metadata, Refund};
use super::TilledClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct CreateRefundRequest {
    pub payment_intent_id: String,
    pub amount: i64,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

impl TilledClient {
    /// Create a refund
    pub async fn create_refund(
        &self,
        charge_id: String,
        amount_cents: i64,
        currency: Option<String>,
        reason: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<Refund, TilledError> {
        let request = CreateRefundRequest {
            payment_intent_id: charge_id,
            amount: amount_cents,
            currency: currency.unwrap_or_else(|| "usd".to_string()),
            reason,
            metadata,
        };

        self.post("/v1/refunds", &request).await
    }

    /// Get a refund by ID
    pub async fn get_refund(&self, refund_id: &str) -> Result<Refund, TilledError> {
        let path = format!("/v1/refunds/{}", refund_id);
        self.get(&path, None).await
    }

    /// List refunds with optional filters
    pub async fn list_refunds(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<Refund>, TilledError> {
        self.get("/v1/refunds", filters).await
    }
}
