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
pub mod stub;

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
        "stub" => Some(Box::new(stub::StubCarrierProvider)),
        _ => None,
    }
}

// ── Unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_stub() {
        let p = get_provider("stub");
        assert!(p.is_some());
        assert_eq!(p.unwrap().carrier_code(), "stub");
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        assert!(get_provider("fedex").is_none());
        assert!(get_provider("").is_none());
        assert!(get_provider("unknown-carrier").is_none());
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
