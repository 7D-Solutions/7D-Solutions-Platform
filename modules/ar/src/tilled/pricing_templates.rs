use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::Deserialize;
use std::collections::HashMap;

/// Pricing template response from Tilled API.
/// Uses `serde_json::Value` for payment-method-specific fee structures
/// (card, eft_debit, etc.) since these vary by template type.
#[derive(Debug, Clone, Deserialize)]
pub struct PricingTemplate {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub payment_method_type: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub account_monthly_fee: Option<i64>,
    #[serde(default)]
    pub account_monthly_minimum_fee: Option<i64>,
    /// Card-specific fee structure (present when payment_method_type = "card").
    #[serde(default)]
    pub card: Option<serde_json::Value>,
    /// EFT debit fee structure (present when payment_method_type = "eft_debit").
    #[serde(default)]
    pub eft_debit: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl TilledClient {
    /// List pricing templates. Typically requires partner scope.
    pub async fn list_pricing_templates(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<PricingTemplate>, TilledError> {
        self.get("/v1/pricing-templates", filters).await
    }

    /// Get a pricing template by ID. Typically requires partner scope.
    pub async fn get_pricing_template(&self, id: &str) -> Result<PricingTemplate, TilledError> {
        let path = format!("/v1/pricing-templates/{id}");
        self.get(&path, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_template_deserializes_card() {
        let value = serde_json::json!({
            "id": "pt_123",
            "name": "Standard Card Rate",
            "status": "active",
            "currency": "usd",
            "payment_method_type": "card",
            "card": {"markups": [{"rate": 2.9, "card_type": "visa"}]},
            "account_monthly_fee": 0,
            "created_at": "2026-01-01T00:00:00Z"
        });
        let pt: PricingTemplate = serde_json::from_value(value).expect("test fixture");
        assert_eq!(pt.id, "pt_123");
        assert_eq!(pt.payment_method_type.as_deref(), Some("card"));
        assert!(pt.card.is_some());
        assert!(pt.eft_debit.is_none());
    }

    #[test]
    fn pricing_template_deserializes_eft() {
        let value = serde_json::json!({
            "id": "pt_456",
            "status": "active",
            "currency": "cad",
            "payment_method_type": "eft_debit",
            "eft_debit": {"transaction_fee": 25, "return_fee": 1000}
        });
        let pt: PricingTemplate = serde_json::from_value(value).expect("test fixture");
        assert_eq!(pt.id, "pt_456");
        assert!(pt.eft_debit.is_some());
        assert!(pt.card.is_none());
    }

    #[test]
    fn pricing_template_deserializes_minimal() {
        let value = serde_json::json!({"id": "pt_min"});
        let pt: PricingTemplate = serde_json::from_value(value).expect("test fixture");
        assert_eq!(pt.id, "pt_min");
        assert!(pt.name.is_none());
        assert!(pt.status.is_none());
    }
}
