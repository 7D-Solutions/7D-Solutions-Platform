use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::Deserialize;
use std::collections::HashMap;

/// Platform fee response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformFee {
    pub id: String,
    #[serde(default)]
    pub amount: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub charge_id: Option<String>,
    #[serde(default)]
    pub payment_intent_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl TilledClient {
    /// List platform fees with optional filters.
    pub async fn list_platform_fees(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<PlatformFee>, TilledError> {
        self.get("/v1/platform-fees", filters).await
    }

    /// Get a platform fee by ID.
    pub async fn get_platform_fee(&self, id: &str) -> Result<PlatformFee, TilledError> {
        let path = format!("/v1/platform-fees/{id}");
        self.get(&path, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_fee_deserializes_full() {
        let value = serde_json::json!({
            "id": "pf_123",
            "amount": 500,
            "currency": "usd",
            "account_id": "acct_456",
            "charge_id": "ch_789",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let fee: PlatformFee = serde_json::from_value(value).unwrap();
        assert_eq!(fee.id, "pf_123");
        assert_eq!(fee.amount, Some(500));
        assert_eq!(fee.currency.as_deref(), Some("usd"));
    }

    #[test]
    fn platform_fee_deserializes_minimal() {
        let value = serde_json::json!({"id": "pf_min"});
        let fee: PlatformFee = serde_json::from_value(value).unwrap();
        assert_eq!(fee.id, "pf_min");
        assert!(fee.amount.is_none());
    }
}
