use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::Deserialize;
use std::collections::HashMap;

/// Product code response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct ProductCode {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub payment_method_type: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl TilledClient {
    /// List available product codes with optional filters.
    pub async fn list_product_codes(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<ProductCode>, TilledError> {
        self.get("/v1/product-codes", filters).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_code_deserializes_full() {
        let value = serde_json::json!({
            "id": "pc_123",
            "name": "Card Processing",
            "description": "Standard card processing",
            "payment_method_type": "card",
            "currency": "usd",
            "country": "US",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let code: ProductCode = serde_json::from_value(value).unwrap();
        assert_eq!(code.id, "pc_123");
        assert_eq!(code.name.as_deref(), Some("Card Processing"));
        assert_eq!(code.payment_method_type.as_deref(), Some("card"));
    }

    #[test]
    fn product_code_deserializes_minimal() {
        let value = serde_json::json!({"id": "pc_min"});
        let code: ProductCode = serde_json::from_value(value).unwrap();
        assert_eq!(code.id, "pc_min");
        assert!(code.name.is_none());
    }
}
