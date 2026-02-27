use super::error::TilledError;
use super::types::{ListResponse, Metadata};
use super::TilledClient;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEndpoint {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled_events: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateWebhookEndpointRequest {
    pub url: String,
    pub enabled_events: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Serialize)]
pub struct UpdateWebhookEndpointRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_events: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl TilledClient {
    /// Create a webhook endpoint.
    pub async fn create_webhook_endpoint(
        &self,
        url: String,
        enabled_events: Vec<String>,
        description: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<WebhookEndpoint, TilledError> {
        let request = CreateWebhookEndpointRequest {
            url,
            enabled_events,
            description,
            metadata,
        };
        self.post("/v1/webhook-endpoints", &request).await
    }

    /// List all webhook endpoints.
    pub async fn list_webhook_endpoints(
        &self,
    ) -> Result<ListResponse<WebhookEndpoint>, TilledError> {
        self.get("/v1/webhook-endpoints", None).await
    }

    /// Get a webhook endpoint by ID.
    pub async fn get_webhook_endpoint(
        &self,
        endpoint_id: &str,
    ) -> Result<WebhookEndpoint, TilledError> {
        let path = format!("/v1/webhook-endpoints/{endpoint_id}");
        self.get(&path, None).await
    }

    /// Update a webhook endpoint.
    pub async fn update_webhook_endpoint(
        &self,
        endpoint_id: &str,
        url: Option<String>,
        enabled_events: Option<Vec<String>>,
        description: Option<String>,
    ) -> Result<WebhookEndpoint, TilledError> {
        let path = format!("/v1/webhook-endpoints/{endpoint_id}");
        let request = UpdateWebhookEndpointRequest {
            url,
            enabled_events,
            description,
        };
        self.patch(&path, &request).await
    }

    /// Delete a webhook endpoint.
    /// Tilled may return 204 with empty body for successful deletes.
    pub async fn delete_webhook_endpoint(&self, endpoint_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/webhook-endpoints/{endpoint_id}");
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
    fn create_webhook_endpoint_payload_serializes_expected_fields() {
        let payload = CreateWebhookEndpointRequest {
            url: "https://example.com/webhook".to_string(),
            enabled_events: vec!["payment_intent.succeeded".to_string()],
            description: Some("Test endpoint".to_string()),
            metadata: None,
        };

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value.get("url").unwrap(), "https://example.com/webhook");
        assert_eq!(
            value.get("enabled_events").unwrap()[0],
            "payment_intent.succeeded"
        );
        assert_eq!(value.get("description").unwrap(), "Test endpoint");
        assert!(value.get("metadata").is_none());
    }

    #[test]
    fn update_webhook_endpoint_payload_omits_none_fields() {
        let payload = UpdateWebhookEndpointRequest {
            url: Some("https://example.com/new".to_string()),
            enabled_events: None,
            description: None,
        };

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value.get("url").unwrap(), "https://example.com/new");
        assert!(value.get("enabled_events").is_none());
        assert!(value.get("description").is_none());
    }

    #[test]
    fn webhook_endpoint_deserializes_with_optional_fields() {
        let value = serde_json::json!({
            "id": "we_123",
            "url": "https://example.com/webhook",
            "status": "enabled",
            "enabled_events": ["payment_intent.succeeded"],
            "secret": "whsec_test"
        });
        let endpoint: WebhookEndpoint = serde_json::from_value(value).unwrap();
        assert_eq!(endpoint.id, "we_123");
        assert_eq!(endpoint.url, "https://example.com/webhook");
        assert_eq!(endpoint.status.as_deref(), Some("enabled"));
        assert_eq!(endpoint.secret.as_deref(), Some("whsec_test"));
        assert!(endpoint.description.is_none());
    }

    #[test]
    fn webhook_endpoint_deserializes_minimal() {
        let value = serde_json::json!({
            "id": "we_456",
            "url": "https://example.com/hook"
        });
        let endpoint: WebhookEndpoint = serde_json::from_value(value).unwrap();
        assert_eq!(endpoint.id, "we_456");
        assert!(endpoint.enabled_events.is_none());
        assert!(endpoint.secret.is_none());
    }
}
