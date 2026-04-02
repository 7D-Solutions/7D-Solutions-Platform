/// Typed tenant-registry client adapter.
///
/// Wraps `platform-client-tenant-registry` generated TenantsClient and maps
/// HTTP status codes to semantic error variants for billing-run error handling.
///
/// Endpoint: GET {base_url}/api/tenants/{tenant_id}/app-id
///
/// Fail-closed: billing run aborts if app_id cannot be resolved.
use platform_client_tenant_registry::TenantsClient;
use platform_sdk::{ClientError, PlatformClient, VerifiedClaims};
use uuid::Uuid;

/// Error from the tenant-registry client.
#[derive(Debug, thiserror::Error)]
pub enum TenantRegistryError {
    #[error("tenant {0} not found in registry")]
    TenantNotFound(Uuid),

    #[error("tenant {0} has no app_id assigned")]
    NoAppId(Uuid),

    #[error("tenant-registry client error: {0}")]
    Client(#[from] ClientError),
}

/// Lightweight HTTP client for resolving tenant_id -> app_id.
///
/// Uses the generated `TenantsClient` from `platform-client-tenant-registry`.
pub struct TenantRegistryClient {
    inner: TenantsClient,
}

impl platform_sdk::PlatformService for TenantRegistryClient {
    const SERVICE_NAME: &'static str = "platform-client-tenant-registry";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self {
            inner: TenantsClient::new(client),
        }
    }
}

impl TenantRegistryClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            inner: TenantsClient::new(PlatformClient::new(base_url.into())),
        }
    }

    /// Resolve tenant_id to app_id.
    ///
    /// Returns `Ok(app_id)` on success.
    /// Returns `Err(TenantRegistryError::TenantNotFound)` if tenant is unknown.
    /// Returns `Err(TenantRegistryError::NoAppId)` if app_id is not yet assigned.
    pub async fn get_app_id(&self, claims: &VerifiedClaims, tenant_id: Uuid) -> Result<String, TenantRegistryError> {
        match self.inner.get_app_id(claims, tenant_id).await {
            Ok(row) => Ok(row.app_id),
            Err(ClientError::Api { status: 404, .. })
            | Err(ClientError::Unexpected { status: 404, .. }) => {
                Err(TenantRegistryError::TenantNotFound(tenant_id))
            }
            Err(ClientError::Api { status: 409, .. })
            | Err(ClientError::Unexpected { status: 409, .. }) => {
                Err(TenantRegistryError::NoAppId(tenant_id))
            }
            Err(e) => Err(TenantRegistryError::Client(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constructs_without_panic() {
        let _client = TenantRegistryClient::new("http://localhost:8092");
    }
}
