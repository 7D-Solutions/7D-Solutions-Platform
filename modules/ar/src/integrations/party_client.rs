//! HTTP client for Party Master cross-module validation.
//!
//! Verifies that a party_id exists in Party Master and belongs to the same tenant
//! before allowing AR operations (invoice/subscription create) to proceed.
//!
//! ## Configuration
//! - `PARTY_MASTER_URL`: Party Master base URL (default: `http://7d-party:8098`)
//!
//! ## Error handling
//! - 404 from Party Master → [`PartyClientError::NotFound`] → caller returns 422
//! - app_id mismatch → [`PartyClientError::TenantMismatch`] → caller returns 422
//! - network error / non-2xx → [`PartyClientError::ServiceUnavailable`] → caller returns 503

use serde::Deserialize;
use std::env;
use uuid::Uuid;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug)]
pub enum PartyClientError {
    NotFound(Uuid),
    TenantMismatch { expected: String, got: String },
    ServiceUnavailable(String),
}

impl std::fmt::Display for PartyClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "Party {} not found in Party Master", id),
            Self::TenantMismatch { expected, got } => {
                write!(
                    f,
                    "Party tenant mismatch: expected {}, got {}",
                    expected, got
                )
            }
            Self::ServiceUnavailable(msg) => {
                write!(f, "Party Master unavailable: {}", msg)
            }
        }
    }
}

// ============================================================================
// Minimal Party Master response shape
// ============================================================================

#[derive(Deserialize)]
struct PartyResponse {
    app_id: String,
}

// ============================================================================
// Public API
// ============================================================================

/// Read the configured Party Master base URL.
pub fn party_master_url() -> String {
    env::var("PARTY_MASTER_URL").unwrap_or_else(|_| "http://7d-party:8098".to_string())
}

/// Verify a party exists in Party Master and belongs to `app_id`.
///
/// Calls `GET {base_url}/api/party/parties/{party_id}` with `X-App-Id: {app_id}`.
///
/// Returns:
/// - `Ok(())` — party exists and app_id matches
/// - `Err(NotFound)` — party not found (404)
/// - `Err(TenantMismatch)` — party found but different app_id
/// - `Err(ServiceUnavailable)` — network error or unexpected HTTP status
pub async fn verify_party(
    base_url: &str,
    party_id: Uuid,
    app_id: &str,
) -> Result<(), PartyClientError> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/party/parties/{}", base_url, party_id);

    let resp = client
        .get(&url)
        .header("x-app-id", app_id)
        .send()
        .await
        .map_err(|e| PartyClientError::ServiceUnavailable(e.to_string()))?;

    let status = resp.status();

    if status.as_u16() == 404 {
        return Err(PartyClientError::NotFound(party_id));
    }

    if !status.is_success() {
        return Err(PartyClientError::ServiceUnavailable(format!(
            "HTTP {}",
            status
        )));
    }

    let party: PartyResponse = resp
        .json()
        .await
        .map_err(|e| PartyClientError::ServiceUnavailable(e.to_string()))?;

    if party.app_id != app_id {
        return Err(PartyClientError::TenantMismatch {
            expected: app_id.to_string(),
            got: party.app_id,
        });
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_party_master_url_defaults() {
        // When env var is not set, returns the Docker service default
        // (We can't unset env vars reliably in tests, but we verify the function returns a string)
        let url = party_master_url();
        assert!(!url.is_empty());
        assert!(url.starts_with("http"));
    }

    #[test]
    fn test_party_client_error_display() {
        let id = Uuid::new_v4();
        let err = PartyClientError::NotFound(id);
        assert!(err.to_string().contains("not found"));

        let err = PartyClientError::TenantMismatch {
            expected: "tenant-a".to_string(),
            got: "tenant-b".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("tenant-a"));
        assert!(msg.contains("tenant-b"));

        let err = PartyClientError::ServiceUnavailable("connection refused".to_string());
        assert!(err.to_string().contains("unavailable"));
    }

    #[tokio::test]
    async fn test_party_verify_unreachable_url_returns_service_unavailable() {
        // Port 19999 should not be listening — confirms connection error maps correctly
        let result = verify_party("http://127.0.0.1:19999", Uuid::new_v4(), "test-app").await;
        assert!(
            matches!(result, Err(PartyClientError::ServiceUnavailable(_))),
            "expected ServiceUnavailable, got {:?}",
            result
        );
    }

    /// Integration test — only runs when Party Master is reachable.
    ///
    /// Set PARTY_MASTER_URL to a running Party Master instance and
    /// PARTY_INTEGRATION_TEST=1 to enable.
    #[tokio::test]
    async fn test_party_verify_invalid_id_returns_not_found() {
        let run = std::env::var("PARTY_INTEGRATION_TEST").unwrap_or_default();
        if run != "1" {
            return; // skip when Party Master is not provisioned
        }

        let url = party_master_url();
        let random_id = Uuid::new_v4();

        let result = verify_party(&url, random_id, "test-app").await;
        assert!(
            matches!(result, Err(PartyClientError::NotFound(_))),
            "expected NotFound for unknown UUID, got {:?}",
            result
        );
    }
}
