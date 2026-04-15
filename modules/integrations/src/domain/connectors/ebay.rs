//! eBay marketplace connector.
//!
//! Implements the platform `Connector` trait for the eBay Fulfillment API.
//! This connector supports order ingestion via polling (pull pattern), using
//! OAuth2 client-credentials for authentication.
//!
//! # Config schema (required keys)
//! - `client_id`     — eBay OAuth application client ID.
//! - `client_secret` — eBay OAuth application client secret.
//! - `ru_name`       — eBay RuName (redirect URI name) for the application.
//! - `environment`   — `"PRODUCTION"` or `"SANDBOX"`.
//!
//! # Test action
//! Validates that all required config fields are present and non-empty, and
//! that `environment` is one of the accepted values.
//! Synchronous — no HTTP calls.

use chrono::Utc;

use super::{
    ConfigField, ConfigFieldType, ConnectorCapabilities, ConnectorError, TestActionResult,
};

// ── Required config field names ───────────────────────────────────────────────

const REQUIRED_FIELDS: &[&str] = &["client_id", "client_secret", "ru_name", "environment"];

const VALID_ENVIRONMENTS: &[&str] = &["PRODUCTION", "SANDBOX"];

// ── Connector implementation ──────────────────────────────────────────────────

/// eBay Fulfillment API connector — zero-state, tenant config passed at invocation.
pub struct EbayConnector;

impl super::Connector for EbayConnector {
    fn connector_type(&self) -> &str {
        "ebay"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            connector_type: "ebay".to_string(),
            description: "eBay marketplace connector — ingest orders via scheduled polling. \
                          Uses OAuth2 client-credentials for authentication."
                .to_string(),
            supports_test_action: true,
            config_fields: vec![
                ConfigField {
                    key: "client_id".to_string(),
                    label: "Client ID".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "client_secret".to_string(),
                    label: "Client Secret".to_string(),
                    field_type: ConfigFieldType::Secret,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "ru_name".to_string(),
                    label: "RuName (Redirect URI Name)".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: None,
                },
                ConfigField {
                    key: "environment".to_string(),
                    label: "Environment (PRODUCTION or SANDBOX)".to_string(),
                    field_type: ConfigFieldType::Text,
                    required: true,
                    default: Some("SANDBOX".to_string()),
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

        let environment = config
            .get("environment")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !VALID_ENVIRONMENTS.contains(&environment) {
            return Err(ConnectorError::InvalidConfig(format!(
                "environment must be one of: {}; got: {}",
                VALID_ENVIRONMENTS.join(", "),
                environment
            )));
        }

        Ok(())
    }

    /// Synchronous test action — validates config fields are present and non-empty.
    ///
    /// No HTTP calls are made here. Real eBay OAuth connectivity is verified in
    /// integration tests that require eBay sandbox credentials.
    fn run_test_action(
        &self,
        config: &serde_json::Value,
        idempotency_key: &str,
    ) -> Result<TestActionResult, ConnectorError> {
        self.validate_config(config)?;

        let environment = config
            .get("environment")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        Ok(TestActionResult {
            connector_type: self.connector_type().to_string(),
            success: true,
            output: serde_json::json!({
                "message": "eBay config validated — all required fields present.",
                "environment": environment,
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
            "client_id": "SomeApp-SomeApp-SBX-abc12def3-abc12345",
            "client_secret": "SBX-abc12def3abc45678-abc12345-abcd1234",
            "ru_name": "Some_App-SomeApp-SomeAp-abcdefgh",
            "environment": "SANDBOX",
        })
    }

    // ── Connector trait ───────────────────────────────────────────────────────

    #[test]
    fn connector_type_is_ebay() {
        assert_eq!(EbayConnector.connector_type(), "ebay");
    }

    #[test]
    fn capabilities_describes_all_required_fields() {
        let caps = EbayConnector.capabilities();
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
        assert!(EbayConnector.validate_config(&valid_config()).is_ok());
    }

    #[test]
    fn valid_production_config_passes_validation() {
        let mut cfg = valid_config();
        cfg["environment"] = json!("PRODUCTION");
        assert!(EbayConnector.validate_config(&cfg).is_ok());
    }

    #[test]
    fn missing_client_id_fails_validation() {
        let mut cfg = valid_config();
        cfg.as_object_mut()
            .expect("config must be object")
            .remove("client_id");
        let err = EbayConnector.validate_config(&cfg).unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
        assert!(err.to_string().contains("client_id"));
    }

    #[test]
    fn missing_client_secret_fails_validation() {
        let mut cfg = valid_config();
        cfg.as_object_mut()
            .expect("config must be object")
            .remove("client_secret");
        let err = EbayConnector.validate_config(&cfg).unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
        assert!(err.to_string().contains("client_secret"));
    }

    #[test]
    fn missing_ru_name_fails_validation() {
        let mut cfg = valid_config();
        cfg.as_object_mut()
            .expect("config must be object")
            .remove("ru_name");
        let err = EbayConnector.validate_config(&cfg).unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
        assert!(err.to_string().contains("ru_name"));
    }

    #[test]
    fn invalid_environment_fails_validation() {
        let mut cfg = valid_config();
        cfg["environment"] = json!("STAGING");
        let err = EbayConnector.validate_config(&cfg).unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
        assert!(err.to_string().contains("environment"));
    }

    #[test]
    fn empty_field_fails_validation() {
        let mut cfg = valid_config();
        cfg["client_id"] = json!("   ");
        let err = EbayConnector.validate_config(&cfg).unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
    }

    #[test]
    fn multiple_missing_fields_listed_in_error() {
        let cfg = json!({});
        let err = EbayConnector.validate_config(&cfg).unwrap_err();
        let msg = err.to_string();
        for &field in REQUIRED_FIELDS {
            assert!(
                msg.contains(field),
                "missing field '{}' not in error: {}",
                field,
                msg
            );
        }
    }

    // ── run_test_action ───────────────────────────────────────────────────────

    #[test]
    fn test_action_with_valid_config_succeeds() {
        let result = EbayConnector
            .run_test_action(&valid_config(), "idem-key-001")
            .expect("test action failed");

        assert!(result.success);
        assert_eq!(result.connector_type, "ebay");
        assert_eq!(result.idempotency_key, "idem-key-001");
        assert_eq!(
            result.output["environment"]
                .as_str()
                .expect("environment should be a string"),
            "SANDBOX"
        );
    }

    #[test]
    fn test_action_with_invalid_config_returns_error() {
        let err = EbayConnector
            .run_test_action(&json!({}), "idem-key-002")
            .unwrap_err();
        assert!(matches!(err, ConnectorError::InvalidConfig(_)));
    }

    // ── Registry ─────────────────────────────────────────────────────────────

    #[test]
    fn registry_returns_ebay_connector() {
        let connector = get_connector("ebay");
        assert!(
            connector.is_some(),
            "get_connector('ebay') should return Some"
        );
        assert_eq!(connector.unwrap().connector_type(), "ebay");
    }

    #[test]
    fn all_connectors_includes_ebay() {
        let caps = all_connectors();
        let types: Vec<&str> = caps.iter().map(|c| c.connector_type.as_str()).collect();
        assert!(
            types.contains(&"ebay"),
            "'ebay' not found in all_connectors(): {:?}",
            types
        );
    }
}
