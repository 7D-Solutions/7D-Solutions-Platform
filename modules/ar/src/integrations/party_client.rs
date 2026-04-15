//! Typed client wrapper for Party Master cross-module validation.
//!
//! Uses [`platform_client_party::PartiesClient`] to verify that a party_id
//! exists in Party Master and belongs to the correct tenant.
//!
//! The `PartiesClient` is constructed at startup from `[platform.services]`
//! in `module.toml` and injected via axum `Extension`.

use platform_sdk::{ClientError, VerifiedClaims};
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

/// Verify a party exists in Party Master and belongs to `app_id`.
///
/// Uses the SDK-wired `PartiesClient` to call `GET /api/party/parties/{id}`,
/// then checks that the returned party's `app_id` matches the expected tenant.
///
/// Returns:
/// - `Ok(())` — party exists and app_id matches
/// - `Err(NotFound)` — party not found (404)
/// - `Err(TenantMismatch)` — party found but different app_id
/// - `Err(ServiceUnavailable)` — network error or unexpected HTTP status
pub async fn verify_party(
    client: &platform_client_party::PartiesClient,
    party_id: Uuid,
    app_id: &str,
    claims: &VerifiedClaims,
) -> Result<(), PartyClientError> {
    let view = client
        .get_party(claims, party_id)
        .await
        .map_err(|e| match &e {
            ClientError::Api { status, .. } if *status == 404 => {
                PartyClientError::NotFound(party_id)
            }
            ClientError::Network(_) => PartyClientError::ServiceUnavailable(e.to_string()),
            _ => PartyClientError::ServiceUnavailable(e.to_string()),
        })?;

    if view.party.app_id != app_id {
        return Err(PartyClientError::TenantMismatch {
            expected: app_id.to_string(),
            got: view.party.app_id,
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
    use platform_sdk::PlatformClient;

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
        let client = platform_client_party::PartiesClient::new(PlatformClient::new(
            "http://127.0.0.1:19999".to_string(),
        ));
        let claims = PlatformClient::service_claims(Uuid::new_v4());
        let result = verify_party(&client, Uuid::new_v4(), "test-app", &claims).await;
        assert!(
            matches!(result, Err(PartyClientError::ServiceUnavailable(_))),
            "expected ServiceUnavailable, got {:?}",
            result
        );
    }
}
