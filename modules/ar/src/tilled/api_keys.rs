use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// API key response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiKey {
    pub id: String,
    #[serde(default, rename = "type")]
    pub key_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub last_four: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Request body for creating an API key.
#[derive(Debug, Serialize)]
pub struct CreateApiKeyRequest {
    #[serde(rename = "type")]
    pub key_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Request body for updating an API key.
#[derive(Debug, Serialize)]
pub struct UpdateApiKeyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl TilledClient {
    /// Create a new API key.
    pub async fn create_api_key(
        &self,
        key_type: String,
        name: Option<String>,
    ) -> Result<ApiKey, TilledError> {
        let request = CreateApiKeyRequest { key_type, name };
        self.post("/v1/api-keys", &request).await
    }

    /// List API keys with optional filters.
    pub async fn list_api_keys(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<ApiKey>, TilledError> {
        self.get("/v1/api-keys", filters).await
    }

    /// Update an API key's name.
    pub async fn update_api_key(
        &self,
        key_id: &str,
        name: Option<String>,
    ) -> Result<ApiKey, TilledError> {
        let path = format!("/v1/api-keys/{key_id}");
        let request = UpdateApiKeyRequest { name };
        self.patch(&path, &request).await
    }

    /// Delete an API key.
    /// Tilled may return 204 with empty body for successful deletes.
    pub async fn delete_api_key(&self, key_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/api-keys/{key_id}");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .delete(&url)
            .headers(self.build_auth_headers()?)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            Err(TilledError::ApiError {
                status_code,
                message,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_deserializes_full() {
        let value = serde_json::json!({
            "id": "ak_123",
            "type": "publishable",
            "name": "Frontend Key",
            "last_four": "abcd",
            "account_id": "acct_456",
            "scope": "merchant",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let key: ApiKey = serde_json::from_value(value).unwrap();
        assert_eq!(key.id, "ak_123");
        assert_eq!(key.key_type.as_deref(), Some("publishable"));
        assert_eq!(key.name.as_deref(), Some("Frontend Key"));
        assert_eq!(key.last_four.as_deref(), Some("abcd"));
    }

    #[test]
    fn api_key_deserializes_minimal() {
        let value = serde_json::json!({"id": "ak_min"});
        let key: ApiKey = serde_json::from_value(value).unwrap();
        assert_eq!(key.id, "ak_min");
        assert!(key.key_type.is_none());
        assert!(key.name.is_none());
    }

    #[test]
    fn create_request_serializes() {
        let req = CreateApiKeyRequest {
            key_type: "publishable".to_string(),
            name: Some("Test Key".to_string()),
        };
        let value = serde_json::to_value(req).unwrap();
        assert_eq!(value.get("type").unwrap(), "publishable");
        assert_eq!(value.get("name").unwrap(), "Test Key");
    }

    #[test]
    fn create_request_omits_none_name() {
        let req = CreateApiKeyRequest {
            key_type: "secret".to_string(),
            name: None,
        };
        let value = serde_json::to_value(req).unwrap();
        assert_eq!(value.get("type").unwrap(), "secret");
        assert!(value.get("name").is_none());
    }

    #[test]
    fn update_request_omits_none_name() {
        let req = UpdateApiKeyRequest { name: None };
        let value = serde_json::to_value(req).unwrap();
        assert!(value.get("name").is_none());
    }
}
