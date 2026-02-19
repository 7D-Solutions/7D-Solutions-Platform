/// HTTP client for the tenant-registry app_id mapping endpoint.
///
/// Resolves tenant_id (UUID) → app_id (VARCHAR) for AR invoice scoping.
/// Called once per billing run; result is used for all AR calls within the run.
///
/// Endpoint: GET {base_url}/api/tenants/{tenant_id}/app-id
///
/// Fail-closed: billing run aborts if app_id cannot be resolved.

use serde::Deserialize;
use uuid::Uuid;

/// Error from the tenant-registry client.
#[derive(Debug, thiserror::Error)]
pub enum TenantRegistryError {
    #[error("tenant {0} not found in registry")]
    TenantNotFound(Uuid),

    #[error("tenant {0} has no app_id assigned")]
    NoAppId(Uuid),

    #[error("tenant-registry HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("tenant-registry returned unexpected status {status} for tenant {tenant_id}")]
    UnexpectedStatus { tenant_id: Uuid, status: u16 },
}

/// Response body from GET /api/tenants/:tenant_id/app-id
#[derive(Debug, Deserialize)]
pub struct AppIdResponse {
    pub app_id: String,
    pub product_code: Option<String>,
}

/// Lightweight HTTP client for resolving tenant_id → app_id.
#[derive(Clone)]
pub struct TenantRegistryClient {
    http: reqwest::Client,
    base_url: String,
}

impl TenantRegistryClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build reqwest client for tenant-registry");

        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Resolve tenant_id to app_id.
    ///
    /// Returns `Ok(app_id)` on success.
    /// Returns `Err(TenantRegistryError::TenantNotFound)` if tenant is unknown.
    /// Returns `Err(TenantRegistryError::NoAppId)` if app_id is not yet assigned.
    pub async fn get_app_id(&self, tenant_id: Uuid) -> Result<String, TenantRegistryError> {
        let url = format!("{}/api/tenants/{}/app-id", self.base_url, tenant_id);

        let response = self.http.get(&url).send().await?;

        match response.status().as_u16() {
            200 => {
                let body: AppIdResponse = response.json().await?;
                Ok(body.app_id)
            }
            404 => Err(TenantRegistryError::TenantNotFound(tenant_id)),
            409 => Err(TenantRegistryError::NoAppId(tenant_id)),
            status => Err(TenantRegistryError::UnexpectedStatus {
                tenant_id,
                status,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_strips_trailing_slash_from_base_url() {
        let client = TenantRegistryClient::new("http://localhost:8092/");
        assert_eq!(client.base_url, "http://localhost:8092");
    }

    #[test]
    fn client_preserves_url_without_trailing_slash() {
        let client = TenantRegistryClient::new("http://localhost:8092");
        assert_eq!(client.base_url, "http://localhost:8092");
    }

    /// Integration test: requires TENANT_REGISTRY_URL env var and a running tenant-registry.
    ///
    /// Run with:
    ///   TENANT_REGISTRY_URL=http://localhost:8092 cargo test -p ttp -- --nocapture tenant_registry_client
    #[tokio::test]
    #[ignore]
    async fn integration_get_app_id_unknown_tenant_returns_not_found() {
        let base_url = std::env::var("TENANT_REGISTRY_URL")
            .unwrap_or_else(|_| "http://localhost:8092".to_string());
        let client = TenantRegistryClient::new(base_url);
        let result = client.get_app_id(Uuid::new_v4()).await;
        assert!(
            matches!(result, Err(TenantRegistryError::TenantNotFound(_))),
            "expected TenantNotFound, got: {:?}",
            result
        );
    }
}
