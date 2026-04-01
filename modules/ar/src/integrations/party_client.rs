//! Typed client wrapper for Party Master cross-module validation.
//!
//! Uses [`platform_client_party::PartiesClient`] to verify that a party_id
//! exists in Party Master and belongs to the correct tenant.
//!
//! ## Configuration
//! - `PARTY_MASTER_URL`: Party Master base URL (default: `http://7d-party:8098`)

use platform_sdk::{ClientError, PlatformClient, VerifiedClaims};
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
// Public API
// ============================================================================

/// Read the configured Party Master base URL.
pub fn party_master_url() -> String {
    env::var("PARTY_MASTER_URL").unwrap_or_else(|_| "http://7d-party:8098".to_string())
}

/// Verify a party exists in Party Master and belongs to `app_id`.
///
/// Uses the generated `PartiesClient` to call `GET /api/party/parties/{id}`,
/// then checks that the returned party's `app_id` matches the expected tenant.
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
    claims: &VerifiedClaims,
) -> Result<(), PartyClientError> {
    let token = security::get_service_token()
        .map_err(|e| PartyClientError::ServiceUnavailable(format!("service token: {e}")))?;
    let client = platform_client_party::PartiesClient::new(
        PlatformClient::new(base_url.to_string()).with_bearer_token(token),
    );

    let view = client.get_party(claims, party_id).await.map_err(|e| match &e {
        ClientError::Api { status, .. } if *status == 404 => PartyClientError::NotFound(party_id),
        ClientError::Network(_) => PartyClientError::ServiceUnavailable(e.to_string()),
        _ => PartyClientError::ServiceUnavailable(e.to_string()),
    })?;

    if view._base_party.app_id != app_id {
        return Err(PartyClientError::TenantMismatch {
            expected: app_id.to_string(),
            got: view._base_party.app_id,
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
        let claims = PlatformClient::service_claims(Uuid::new_v4());
        let result = verify_party("http://127.0.0.1:19999", Uuid::new_v4(), "test-app", &claims).await;
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
            return;
        }

        let url = party_master_url();
        let random_id = Uuid::new_v4();

        let claims = PlatformClient::service_claims(Uuid::new_v4());
        let result = verify_party(&url, random_id, "test-app", &claims).await;
        assert!(
            matches!(result, Err(PartyClientError::NotFound(_))),
            "expected NotFound for unknown UUID, got {:?}",
            result
        );
    }
}
