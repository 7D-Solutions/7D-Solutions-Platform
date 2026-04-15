use super::error::TilledError;
use super::types::Metadata;
use super::TilledClient;
use serde::{Deserialize, Serialize};

/// Auth link returned by Tilled's `/v1/auth-links` endpoint.
/// Creates hosted onboarding URLs for merchants.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthLink {
    pub id: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub redeemed: Option<bool>,
    #[serde(default)]
    pub redirect_url: Option<String>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Request body for creating an auth link.
#[derive(Debug, Serialize)]
pub struct CreateAuthLinkRequest {
    pub user_id: String,
    pub expiration: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

impl TilledClient {
    /// Create an auth link for hosted onboarding.
    /// Operates on partner scope.
    pub async fn create_auth_link(
        &self,
        user_id: String,
        expiration: String,
        redirect_url: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<AuthLink, TilledError> {
        let request = CreateAuthLinkRequest {
            user_id,
            expiration,
            redirect_url,
            metadata,
        };
        self.post("/v1/auth-links", &request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_link_deserializes_full_response() {
        let value = serde_json::json!({
            "id": "link_123",
            "account_id": "acct_456",
            "url": "https://sandbox-app.tilled.com/auth/magic-link?key=link_123",
            "user_id": "user_789",
            "expires_at": "2026-02-28T00:00:00Z",
            "redeemed": false,
            "redirect_url": "/dashboard",
            "created_at": "2026-02-27T00:00:00Z",
            "updated_at": "2026-02-27T00:00:00Z"
        });
        let link: AuthLink = serde_json::from_value(value).expect("test fixture");
        assert_eq!(link.id, "link_123");
        assert_eq!(
            link.url.as_deref(),
            Some("https://sandbox-app.tilled.com/auth/magic-link?key=link_123")
        );
        assert_eq!(link.redeemed, Some(false));
    }

    #[test]
    fn auth_link_deserializes_minimal() {
        let value = serde_json::json!({"id": "link_min"});
        let link: AuthLink = serde_json::from_value(value).expect("test fixture");
        assert_eq!(link.id, "link_min");
        assert!(link.url.is_none());
    }

    #[test]
    fn create_auth_link_request_omits_none_fields() {
        let req = CreateAuthLinkRequest {
            user_id: "user_123".to_string(),
            expiration: "1d".to_string(),
            redirect_url: None,
            metadata: None,
        };
        let value = serde_json::to_value(req).expect("test fixture");
        assert_eq!(value.get("user_id").expect("test fixture"), "user_123");
        assert_eq!(value.get("expiration").expect("test fixture"), "1d");
        assert!(value.get("redirect_url").is_none());
        assert!(value.get("metadata").is_none());
    }
}
