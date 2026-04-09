//! Connector contract — the shared interface every integration connector must satisfy.
//!
//! A *connector* is an outbound integration channel that the platform can
//! configure per-tenant and invoke on demand.  All connectors share this
//! contract so the hub can dispatch to them uniformly without bespoke code
//! paths per integration type.
//!
//! # Design principles
//! - **Deterministic test action**: every connector must expose a test action
//!   that produces a stable, predictable result given the same input.  This
//!   lets E2E tests validate the full plumbing without relying on external APIs.
//! - **Config schema self-description**: connectors advertise what fields they
//!   need so UIs and validators can work generically.
//! - **Idempotency key threading**: every test-action invocation carries an
//!   idempotency key so callers can safely retry without double-side-effects.

pub mod amazon;
pub mod ebay;
pub mod echo;
pub mod repo;
pub mod service;
pub mod shopify;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

// ============================================================================
// Config schema self-description
// ============================================================================

/// A single declared config field for a connector.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigField {
    /// Machine-readable field key.
    pub key: String,
    /// Human-readable label.
    pub label: String,
    /// Field data type hint for UIs.
    pub field_type: ConfigFieldType,
    /// Whether the field must be supplied.
    pub required: bool,
    /// Optional default value (serialised as string).
    pub default: Option<String>,
}

/// Supported config field types.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    Text,
    Secret,
    Url,
    Boolean,
    Integer,
}

// ============================================================================
// Capabilities
// ============================================================================

/// What a given connector implementation supports.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConnectorCapabilities {
    /// Connector kind identifier (e.g. `"echo"`, `"http-push"`).
    pub connector_type: String,
    /// Human-readable description of the connector.
    pub description: String,
    /// Whether this connector implements `run_test_action`.
    pub supports_test_action: bool,
    /// Declared config fields (used for config validation and UI rendering).
    pub config_fields: Vec<ConfigField>,
}

// ============================================================================
// Test action result
// ============================================================================

/// Outcome of a connector test action.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestActionResult {
    /// Connector type that executed the action.
    pub connector_type: String,
    /// Whether the action succeeded.
    pub success: bool,
    /// Connector-specific output payload.
    pub output: serde_json::Value,
    /// Idempotency key echoed back from the caller.
    pub idempotency_key: String,
    /// Wall-clock time when the action was executed.
    pub executed_at: DateTime<Utc>,
}

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("Unknown connector type: {0}")]
    UnknownType(String),
    #[error("Config validation failed: {0}")]
    InvalidConfig(String),
    #[error("Test action failed: {0}")]
    ActionFailed(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Not found: {0}")]
    NotFound(String),
}

// ============================================================================
// Connector trait
// ============================================================================

/// The contract every connector implementation must satisfy.
///
/// Implementations are zero-state — config is passed at invocation time so
/// the same connector object can serve all tenants.
pub trait Connector: Send + Sync {
    /// Unique type discriminator matching `integrations_connector_configs.connector_type`.
    fn connector_type(&self) -> &str;

    /// Advertise capabilities and config schema.
    fn capabilities(&self) -> ConnectorCapabilities;

    /// Validate a tenant-supplied config blob against the connector's schema.
    fn validate_config(&self, config: &serde_json::Value) -> Result<(), ConnectorError>;

    /// Run the test action and return a deterministic result.
    ///
    /// The `idempotency_key` is caller-supplied and must be echoed back in
    /// [`TestActionResult::idempotency_key`].
    fn run_test_action(
        &self,
        config: &serde_json::Value,
        idempotency_key: &str,
    ) -> Result<TestActionResult, ConnectorError>;
}

// ============================================================================
// Registry — maps connector_type strings to implementations
// ============================================================================

/// Look up a connector implementation by its type string.
///
/// Returns `None` if the type is not registered.  Use [`all_connectors`] to
/// enumerate what is available.
pub fn get_connector(connector_type: &str) -> Option<Box<dyn Connector>> {
    match connector_type {
        "echo" => Some(Box::new(echo::EchoConnector)),
        "shopify" => Some(Box::new(shopify::ShopifyConnector)),
        "amazon_sp" => Some(Box::new(amazon::AmazonConnector)),
        "ebay" => Some(Box::new(ebay::EbayConnector)),
        _ => None,
    }
}

/// Return all registered connector capability descriptors.
pub fn all_connectors() -> Vec<ConnectorCapabilities> {
    vec![
        echo::EchoConnector.capabilities(),
        shopify::ShopifyConnector.capabilities(),
        amazon::AmazonConnector.capabilities(),
        ebay::EbayConnector.capabilities(),
    ]
}

// ============================================================================
// DB model
// ============================================================================

/// A persisted connector config row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ConnectorConfig {
    pub id: uuid::Uuid,
    pub app_id: String,
    pub connector_type: String,
    pub name: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for registering a connector.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterConnectorRequest {
    /// Connector type (must be a registered type, e.g. `"echo"`).
    pub connector_type: String,
    /// Human-readable name for this registration.
    pub name: String,
    /// Connector-specific config. Validated by the connector's own schema.
    pub config: Option<serde_json::Value>,
}

/// Request body for invoking the test action.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RunTestActionRequest {
    /// Caller-supplied idempotency key.
    pub idempotency_key: String,
}
