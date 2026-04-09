//! Amazon SP-API marketplace connector.
//!
//! Implements the platform `Connector` trait for Amazon Selling Partner API.
//! This connector supports order ingestion via polling (pull pattern), in
//! contrast to Shopify's webhook push pattern.
//!
//! # Config schema (required keys)
//! - `seller_id`       — Amazon Seller Central merchant identifier.
//! - `refresh_token`   — LWA OAuth refresh token for the seller account.
//! - `client_id`       — LWA application client ID.
//! - `client_secret`   — LWA application client secret.
//! - `marketplace_id`  — Amazon marketplace identifier (e.g. `ATVPDKIKX0DER` for US).
//!
//! # Test action
//! Validates that all required config fields are present and non-empty.
//! Synchronous — no HTTP calls (real SP-API connectivity is verified in
//! integration tests that exchange a live LWA token).

use chrono::Utc;

use super::{
    ConfigField, ConfigFieldType, ConnectorCapabilities, ConnectorError, TestActionResult,
};

// ── Required config field names ───────────────────────────────────────────────

const REQUIRED_FIELDS: &[&str] = &[
    "seller_id",
    "refresh_token",
    "client_id",
    "client_secret",
    "marketplace_id",
];

// ── Connector implementation ──────────────────────────────────────────────────

/// Amazon SP-API connector — zero-state, tenant config passed at invocation.
pub struct AmazonConnector;

impl super::Connector for AmazonConnector {
    fn connector_type(&self) -> &str {
        "amazon_sp"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            connector_type: "amazon_sp".to_string(),
            description: "Amazon Selling Partner API — ingest orders via scheduled polling. \
                          Uses LWA OAuth (refresh_token) for authentication."
                .to_string(),
            supports_test_action: true,
            config_fields: vec![
                ConfigField {
                    key: "seller_id".to_string(),
                    label: "Seller ID".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "refresh_token".to_string(),
                    label: "LWA Refresh Token".to_string(),
                    field_type: ConfigFieldType::Secret,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "client_id".to_string(),
                    label: "LWA Client ID".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "client_secret".to_string(),
                    label: "LWA Client Secret".to_string(),
                    field_type: ConfigFieldType::Secret,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "marketplace_id".to_string(),
                    label: "Marketplace ID".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "last_poll_timestamp".to_string(),
                    label: "Last Poll Timestamp (ISO-8601, managed by platform)".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: false,
                    default: None,
                },
            ],
        }
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), ConnectorError> {
        let mut missing = Vec::new();

        for &field in REQUIRED_FIELDS {
            let value = config.get(field).and_then(|v| v.as_str()).unwrap_or("");
            if value.trim().is_empty() {
                missing.push(field);
            }
        }

        if !missing.is_empty() {
            return Err(ConnectorError::InvalidConfig(format!(
                "missing required fields: {}",
                missing.join(", ")
            )));
        }

        Ok(())
    }

    /// Synchronous test action — validates config fields are present and non-empty.
    ///
    /// No HTTP calls are made here. Real LWA token exchange is verified in
    /// integration tests that require Amazon sandbox credentials.
    fn run_test_action(
        &self,
        config: &serde_json::Value,
        idempotency_key: &str,
    ) -> Result<TestActionResult, ConnectorError> {
        self.validate_config(config)?;

        Ok(TestActionResult {
            connector_type: self.connector_type().to_string(),
            success: true,
            output: serde_json::json!({
                "message": "Amazon SP-API config validated — all required fields present.",
                "marketplace_id": config.get("marketplace_id").and_then(|v| v.as_str()).unwrap_or(""),
            }),
            idempotency_key: idempotency_key.to_string(),
            executed_at: Utc::now(),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connectors::{all_connectors, get_connector, Connector};
    use serde_json::json;

    fn valid_config() -> serde_json::Value {
        json!({
            "seller_id": "A1B2C3D4E5F6G7",
            "refresh_token": "Atzr|test-refresh-token",
            "client_id": "amzn1.application-oa2-client.test",
            "client_secret": "test-client-secret",
            "marketplace_id": "ATVPDKIKX0DER",
        })
    }

    // ── Connector trait ───────────────────────────────────────────────────────

    #[test]
    fn connector_type_is_amazon_sp() {
        assert_eq!(AmazonConnector.connector_type(), "amazon_sp");
    }

    #[test]
    fn capabilities_describes_all_required_fields() {
        let caps = AmazonConnector.capabilities();
        let required_keys: Vec<&str> = caps
            .config_fields
            .iter()
            .filter(|f| f.required)
            .map(|f| f.key.as_str())
            .collect();

        for &field in REQUIRED_FIELDS {
            assert!(
                required_keys.contains(&field),
                "required field '{}' missing from capabilities",
                field
            );
        }
    }

    // ── validate_config ───────────────────────────────────────────────────────

    #[test]
    fn valid_config_passes_validation() {
        assert!(AmazonConnector.validate_config(&valid_config()).is_ok());
    }

    #[test]
    fn missing_seller_id_fails_validation() {
        let mut cfg = valid_config();
        cfg.as_object_mut().expect("config must be an object").remove("seller_id");
        let err = AmazonConnector.validate_config(&cfg).unwrap_err();
        assert!(
            matches!(err, ConnectorError::InvalidConfig(_)),
            "expected InvalidConfig, got: {:?}",
            err
        );
        let msg = err.to_string();
        assert!(msg.contains("seller_id"), "error should mention 'seller_id': {}", msg);
    }

    #[test]
    fn empty_marketplace_id_fails_validation() {
        let mut cfg = valid_config();
        cfg["marketplace_id"] = json!("   ");
        let err = AmazonConnector.validate_config(&cfg).unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
    }

    #[test]
    fn multiple_missing_fields_listed_in_error() {
        let cfg = json!({});
        let err = AmazonConnector.validate_config(&cfg).unwrap_err();
        let msg = err.to_string();
        for &field in REQUIRED_FIELDS {
            assert!(msg.contains(field), "missing field '{}' not in error: {}", field, msg);
        }
    }

    // ── run_test_action ───────────────────────────────────────────────────────

    #[test]
    fn test_action_with_valid_config_succeeds() {
        let result = AmazonConnector
            .run_test_action(&valid_config(), "idem-key-001")
            .expect("test action failed");

        assert!(result.success);
        assert_eq!(result.connector_type, "amazon_sp");
        assert_eq!(result.idempotency_key, "idem-key-001");
        assert_eq!(
            result.output["marketplace_id"].as_str().expect("marketplace_id should be a string"),
            "ATVPDKIKX0DER"
        );
    }

    #[test]
    fn test_action_with_invalid_config_returns_error() {
        let err = AmazonConnector
            .run_test_action(&json!({}), "idem-key-002")
            .unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
    }

    // ── Registry ─────────────────────────────────────────────────────────────

    #[test]
    fn registry_returns_amazon_connector() {
        let connector = get_connector("amazon_sp");
        assert!(connector.is_some(), "get_connector('amazon_sp') should return Some");
        let c = connector.expect("get_connector('amazon_sp') should return Some");
        assert_eq!(c.connector_type(), "amazon_sp");
    }

    #[test]
    fn all_connectors_includes_amazon_sp() {
        let caps = all_connectors();
        let types: Vec<&str> = caps.iter().map(|c| c.connector_type.as_str()).collect();
        assert!(
            types.contains(&"amazon_sp"),
            "'amazon_sp' not found in all_connectors(): {:?}",
            types
        );
    }
}
