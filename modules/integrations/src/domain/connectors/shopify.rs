//! Shopify marketplace connector.
//!
//! Validates shop credentials (shop_domain, api_key, api_secret, webhook_secret)
//! and describes the order-ingestion capabilities. Real connectivity is verified
//! by HMAC-authenticated webhooks processed by `ShopifyNormalizer`.
//!
//! # Config schema
//! ```json
//! {
//!   "shop_domain":    "my-store.myshopify.com",
//!   "api_key":        "<shopify-api-key>",
//!   "api_secret":     "<shopify-api-secret>",
//!   "webhook_secret": "<shopify-webhook-secret>"
//! }
//! ```
//! All fields are required and must be non-empty strings.

use chrono::Utc;
use serde_json::json;

use super::{
    ConfigField, ConfigFieldType, Connector, ConnectorCapabilities, ConnectorError,
    TestActionResult,
};

/// Shopify connector implementation.
///
/// Stateless — no DB or network I/O.  Config is validated at invocation time.
pub struct ShopifyConnector;

const REQUIRED_FIELDS: &[&str] = &["shop_domain", "api_key", "api_secret", "webhook_secret"];

impl Connector for ShopifyConnector {
    fn connector_type(&self) -> &str {
        "shopify"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            connector_type: "shopify".to_string(),
            description:
                "Shopify marketplace connector. Ingests orders via HMAC-verified webhooks \
                 (orders/create, orders/updated) and emits platform-standard order events."
                    .to_string(),
            supports_test_action: true,
            config_fields: vec![
                ConfigField {
                    key: "shop_domain".to_string(),
                    label: "Shop Domain".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    field_type: ConfigFieldType::Secret,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "api_secret".to_string(),
                    label: "API Secret".to_string(),
                    field_type: ConfigFieldType::Secret,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "webhook_secret".to_string(),
                    label: "Webhook Secret".to_string(),
                    field_type: ConfigFieldType::Secret,
                    required: true,
                    default: None,
                },
            ],
        }
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), ConnectorError> {
        for &field in REQUIRED_FIELDS {
            match config.get(field) {
                None => {
                    return Err(ConnectorError::InvalidConfig(format!(
                        "missing required field: {}",
                        field
                    )))
                }
                Some(v) if !v.is_string() => {
                    return Err(ConnectorError::InvalidConfig(format!(
                        "{} must be a string",
                        field
                    )))
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Validate config fields are present and non-empty. Returns a summary payload.
    ///
    /// Synchronous — no HTTP calls. Real connectivity is established when Shopify
    /// delivers an HMAC-signed webhook that the normalizer accepts.
    fn run_test_action(
        &self,
        config: &serde_json::Value,
        idempotency_key: &str,
    ) -> Result<TestActionResult, ConnectorError> {
        self.validate_config(config)?;

        // Ensure all fields are non-empty strings
        for &field in REQUIRED_FIELDS {
            let val = config.get(field).and_then(|v| v.as_str()).unwrap_or("");
            if val.is_empty() {
                return Err(ConnectorError::InvalidConfig(format!(
                    "{} must not be empty",
                    field
                )));
            }
        }

        let shop_domain = config
            .get("shop_domain")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        Ok(TestActionResult {
            connector_type: "shopify".to_string(),
            success: true,
            output: json!({
                "connector_type": "shopify",
                "shop_domain": shop_domain,
                "webhook_events": ["orders/create", "orders/updated"],
                "config_validated": true,
            }),
            idempotency_key: idempotency_key.to_string(),
            executed_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_config() -> serde_json::Value {
        json!({
            "shop_domain": "my-store.myshopify.com",
            "api_key": "key123",
            "api_secret": "secret456",
            "webhook_secret": "wh_secret789",
        })
    }

    #[test]
    fn connector_type_is_shopify() {
        assert_eq!(ShopifyConnector.connector_type(), "shopify");
    }

    #[test]
    fn validate_config_valid() {
        assert!(ShopifyConnector.validate_config(&valid_config()).is_ok());
    }

    #[test]
    fn validate_config_missing_shop_domain_fails() {
        let config = json!({
            "api_key": "k",
            "api_secret": "s",
            "webhook_secret": "w",
        });
        assert!(matches!(
            ShopifyConnector.validate_config(&config),
            Err(ConnectorError::InvalidConfig(_))
        ));
    }

    #[test]
    fn validate_config_missing_webhook_secret_fails() {
        let config = json!({
            "shop_domain": "shop.myshopify.com",
            "api_key": "k",
            "api_secret": "s",
        });
        assert!(matches!(
            ShopifyConnector.validate_config(&config),
            Err(ConnectorError::InvalidConfig(_))
        ));
    }

    #[test]
    fn run_test_action_valid_config_returns_success() {
        let result = ShopifyConnector
            .run_test_action(&valid_config(), "idem-key-001")
            .expect("should succeed");
        assert!(result.success);
        assert_eq!(result.connector_type, "shopify");
        assert_eq!(result.idempotency_key, "idem-key-001");
        assert_eq!(
            result.output.get("shop_domain").and_then(|v| v.as_str()),
            Some("my-store.myshopify.com")
        );
    }

    #[test]
    fn run_test_action_empty_field_fails() {
        let config = json!({
            "shop_domain": "",
            "api_key": "k",
            "api_secret": "s",
            "webhook_secret": "w",
        });
        assert!(matches!(
            ShopifyConnector.run_test_action(&config, "k"),
            Err(ConnectorError::InvalidConfig(_))
        ));
    }

    #[test]
    fn get_connector_returns_shopify() {
        use crate::domain::connectors::get_connector;
        assert!(get_connector("shopify").is_some());
    }

    #[test]
    fn all_connectors_includes_shopify() {
        use crate::domain::connectors::all_connectors;
        let types: Vec<String> = all_connectors()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();
        assert!(types.contains(&"shopify".to_string()));
    }
}
