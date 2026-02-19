use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::PlatformClientError;

const DEFAULT_BASE_URL: &str = "http://7d-identity-auth:8091";

/// HTTP client for the identity-auth service.
#[derive(Debug, Clone)]
pub struct AuthClient {
    base_url: String,
    http: Client,
}

impl AuthClient {
    pub fn new(http: Client) -> Self {
        let base_url = std::env::var("AUTH_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self { base_url, http }
    }

    pub fn with_base_url(http: Client, base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http,
        }
    }

    pub async fn login(
        &self,
        tenant_id: Uuid,
        email: &str,
        password: &str,
    ) -> Result<TokenResponse, PlatformClientError> {
        let url = format!("{}/api/auth/login", self.base_url);
        tracing::debug!(url = %url, tenant_id = %tenant_id, "auth.login");

        let body = LoginRequest {
            tenant_id,
            email: email.to_string(),
            password: password.to_string(),
        };

        let resp = self.http.post(&url).json(&body).send().await?;
        parse_response(resp).await
    }

    pub async fn refresh(
        &self,
        tenant_id: Uuid,
        refresh_token: &str,
    ) -> Result<TokenResponse, PlatformClientError> {
        let url = format!("{}/api/auth/refresh", self.base_url);
        tracing::debug!(url = %url, tenant_id = %tenant_id, "auth.refresh");

        let body = RefreshRequest {
            tenant_id,
            refresh_token: refresh_token.to_string(),
        };

        let resp = self.http.post(&url).json(&body).send().await?;
        parse_response(resp).await
    }
}

async fn parse_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, PlatformClientError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(PlatformClientError::api(status.as_u16(), body));
    }
    let bytes = resp.bytes().await?;
    serde_json::from_slice(&bytes).map_err(|e| {
        PlatformClientError::Deserialization(format!(
            "{}: {}",
            e,
            String::from_utf8_lossy(&bytes)
        ))
    })
}

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Serialize)]
struct LoginRequest {
    pub tenant_id: Uuid,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
struct RefreshRequest {
    pub tenant_id: Uuid,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub token_type: String,
    pub access_token: String,
    pub expires_in_seconds: i64,
    pub refresh_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_response_deserializes() {
        let json = serde_json::json!({
            "token_type": "Bearer",
            "access_token": "eyJ...",
            "expires_in_seconds": 3600,
            "refresh_token": "rt_abc123"
        });
        let resp: TokenResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.token_type, "Bearer");
        assert_eq!(resp.expires_in_seconds, 3600);
    }

    #[test]
    fn login_request_serializes() {
        let req = LoginRequest {
            tenant_id: Uuid::nil(),
            email: "user@example.com".into(),
            password: "secret".into(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["email"], "user@example.com");
        assert!(json.get("tenant_id").is_some());
    }

    #[test]
    fn refresh_request_serializes() {
        let req = RefreshRequest {
            tenant_id: Uuid::nil(),
            refresh_token: "rt_abc".into(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["refresh_token"], "rt_abc");
    }

    #[test]
    fn auth_client_constructs() {
        let http = Client::new();
        let client = AuthClient::with_base_url(http, "http://localhost:8091");
        assert_eq!(client.base_url, "http://localhost:8091");
    }
}
