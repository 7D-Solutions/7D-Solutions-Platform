//! CarrierProvider trait and registry.
//!
//! A carrier provider is a zero-state object that implements async carrier API
//! calls for rate quoting, label creation, and tracking. `config` is passed at
//! call time so the same provider object serves all tenants.
//!
//! Registry follows the same pattern as `get_connector()` in integrations —
//! match on carrier_code string, return `Box<dyn CarrierProvider>`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod credentials;
pub mod dispatch;
pub mod fedex;
pub mod odfl;
pub mod rl;
pub mod stub;
pub mod ups;
pub mod usps;

// ── Response types ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateQuote {
    pub service_level: String,
    pub carrier_code: String,
    pub total_charge_minor: i64,
    pub currency: String,
    pub estimated_days: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelResult {
    pub tracking_number: String,
    pub label_format: String,
    /// Base64-encoded label bytes or a presigned URL.
    pub label_data: String,
    pub carrier_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackingResult {
    pub tracking_number: String,
    pub carrier_code: String,
    pub status: String,
    pub location: Option<String>,
    pub estimated_delivery: Option<String>,
    pub events: Vec<TrackingEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackingEvent {
    pub timestamp: String,
    pub description: String,
    pub location: Option<String>,
}

// ── Error ─────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CarrierProviderError {
    #[error("carrier API error: {0}")]
    ApiError(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("credentials error: {0}")]
    CredentialsError(String),
    #[error("shipment not found: {0}")]
    NotFound(String),
}

/// Resolve the deployment environment name used for carrier warnings.
///
/// `ENVIRONMENT` takes precedence so newer deployments can use the clearer
/// variable name. `ENV` remains supported for existing module manifests.
pub fn deployment_environment() -> String {
    std::env::var("ENVIRONMENT")
        .or_else(|_| std::env::var("ENV"))
        .unwrap_or_else(|_| "development".to_string())
}

/// Return `true` when the deployment is a local development environment.
pub fn is_dev_environment(env: &str) -> bool {
    matches!(env.to_ascii_lowercase().as_str(), "dev" | "development")
}

/// Warn when the module is running outside development.
///
/// The stub is fine for tests and local runs. Production and staging should
/// use a real carrier adapter.
pub fn warn_if_stub_provider_enabled(env: &str) {
    if !is_dev_environment(env) {
        tracing::warn!(
            environment = %env,
            "StubCarrierProvider should not be used outside development; use fedex, ups, or usps for real traffic"
        );
    }
}

// ── Trait ─────────────────────────────────────────────────────

/// Contract every carrier provider implementation must satisfy.
///
/// Implementations are zero-state — `config` carries credentials/settings
/// so the same provider object serves all tenants.
#[async_trait]
pub trait CarrierProvider: Send + Sync {
    /// Unique carrier code matching `CarrierRequest.carrier_code`.
    fn carrier_code(&self) -> &str;

    /// Fetch rate quotes for a shipment.
    async fn get_rates(
        &self,
        req: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<Vec<RateQuote>, CarrierProviderError>;

    /// Create a shipping label.
    async fn create_label(
        &self,
        req: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<LabelResult, CarrierProviderError>;

    /// Track a shipment by tracking number.
    async fn track(
        &self,
        tracking_number: &str,
        config: &serde_json::Value,
    ) -> Result<TrackingResult, CarrierProviderError>;
}

// ── Registry ──────────────────────────────────────────────────

/// Resolve a carrier provider by `carrier_code`.
///
/// Returns `None` if the code is not registered.
pub fn get_provider(carrier_code: &str) -> Option<Box<dyn CarrierProvider>> {
    match carrier_code {
        "fedex" => Some(Box::new(fedex::FedexCarrierProvider)),
        "odfl" => Some(Box::new(odfl::OdflCarrierProvider)),
        "rl" => Some(Box::new(rl::RlCarrierProvider)),
        "stub" => Some(Box::new(stub::StubCarrierProvider)),
        "ups" => Some(Box::new(ups::UpsCarrierProvider::new())),
        "usps" => Some(Box::new(usps::UspsCarrierProvider)),
        _ => None,
    }
}

// ── Unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_environment_detection_accepts_common_aliases() {
        assert!(is_dev_environment("dev"));
        assert!(is_dev_environment("development"));
        assert!(is_dev_environment("DEV"));
        assert!(is_dev_environment("Development"));
    }

    #[test]
    fn non_dev_environment_detection_rejects_other_values() {
        assert!(!is_dev_environment("staging"));
        assert!(!is_dev_environment("production"));
        assert!(!is_dev_environment("local"));
    }

    #[test]
    fn registry_resolves_stub() {
        let p = get_provider("stub");
        assert!(p.is_some());
        assert_eq!(p.unwrap().carrier_code(), "stub");
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        assert!(get_provider("").is_none());
        assert!(get_provider("unknown-carrier").is_none());
        assert!(get_provider("dhl").is_none());
    }

    #[test]
    fn registry_resolves_ups() {
        let p = get_provider("ups");
        assert!(p.is_some());
        assert_eq!(p.unwrap().carrier_code(), "ups");
    }

    #[test]
    fn registry_resolves_fedex() {
        let p = get_provider("fedex");
        assert!(p.is_some());
        assert_eq!(p.unwrap().carrier_code(), "fedex");
    }

    #[tokio::test]
    async fn stub_returns_rate_quotes() {
        let provider = stub::StubCarrierProvider;
        let rates = provider
            .get_rates(&serde_json::json!({}), &serde_json::json!({}))
            .await
            .expect("get_rates failed");
        assert!(!rates.is_empty());
        assert!(rates.iter().all(|r| r.carrier_code == "stub"));
    }

    #[tokio::test]
    async fn stub_returns_label_result() {
        let provider = stub::StubCarrierProvider;
        let label = provider
            .create_label(&serde_json::json!({}), &serde_json::json!({}))
            .await
            .expect("create_label failed");
        assert_eq!(label.carrier_code, "stub");
        assert!(!label.tracking_number.is_empty());
        assert!(!label.label_data.is_empty());
    }

    #[tokio::test]
    async fn stub_returns_tracking_result() {
        let provider = stub::StubCarrierProvider;
        let result = provider
            .track("STUB-TRACK-001", &serde_json::json!({}))
            .await
            .expect("track failed");
        assert_eq!(result.carrier_code, "stub");
        assert_eq!(result.tracking_number, "STUB-TRACK-001");
        assert!(!result.status.is_empty());
    }
}
