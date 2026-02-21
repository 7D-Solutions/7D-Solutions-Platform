//! Echo connector — the built-in test connector.
//!
//! The echo connector is a self-contained integration that requires no
//! external services.  Given an optional `echo_prefix` in its config, the
//! test action returns a deterministic payload containing:
//!
//!   `echo: <prefix> | idempotency: <key>`
//!
//! This makes it ideal for E2E tests: register an echo connector, invoke the
//! test action, and assert the response contains the exact expected string
//! without any external API calls or flaky dependencies.
//!
//! # Config schema
//! ```json
//! { "echo_prefix": "hello" }
//! ```
//! All fields are optional; `echo_prefix` defaults to `"ping"`.

use chrono::Utc;
use serde_json::json;

use super::{
    ConfigField, ConfigFieldType, Connector, ConnectorCapabilities, ConnectorError, TestActionResult,
};

/// Echo connector implementation.
///
/// Stateless — no DB or network I/O.  All behaviour is derived from the
/// per-invocation config blob.
pub struct EchoConnector;

impl Connector for EchoConnector {
    fn connector_type(&self) -> &str {
        "echo"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            connector_type: "echo".to_string(),
            description: "Built-in echo connector for E2E testing. Returns a deterministic \
                          response derived from config without any external API calls."
                .to_string(),
            supports_test_action: true,
            config_fields: vec![ConfigField {
                key: "echo_prefix".to_string(),
                label: "Echo prefix".to_string(),
                field_type: ConfigFieldType::Text,
                required: false,
                default: Some("ping".to_string()),
            }],
        }
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), ConnectorError> {
        if let Some(obj) = config.as_object() {
            if let Some(prefix) = obj.get("echo_prefix") {
                if !prefix.is_string() {
                    return Err(ConnectorError::InvalidConfig(
                        "echo_prefix must be a string".to_string(),
                    ));
                }
                let s = prefix.as_str().unwrap_or("");
                if s.len() > 64 {
                    return Err(ConnectorError::InvalidConfig(
                        "echo_prefix must be \u{2264} 64 characters".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn run_test_action(
        &self,
        config: &serde_json::Value,
        idempotency_key: &str,
    ) -> Result<TestActionResult, ConnectorError> {
        let prefix = config
            .get("echo_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("ping");

        let message = format!("echo: {} | idempotency: {}", prefix, idempotency_key);

        let output = json!({
            "message": message,
            "echo_prefix": prefix,
            "connector_type": "echo"
        });

        Ok(TestActionResult {
            connector_type: "echo".to_string(),
            success: true,
            output,
            idempotency_key: idempotency_key.to_string(),
            executed_at: Utc::now(),
        })
    }
}
