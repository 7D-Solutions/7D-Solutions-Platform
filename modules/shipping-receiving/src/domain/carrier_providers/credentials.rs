//! Credential facade for carrier integrations.
//!
//! Fetches carrier API credentials from the Integrations module via an
//! internal HTTP endpoint. The base URL is configured via `INTEGRATIONS_SERVICE_URL`.
//!
//! Shipping-receiving has no direct access to connector config storage —
//! the Integrations module is the single source of truth for credentials.

use reqwest::Client;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error("carrier credentials not found for connector_type={0}")]
    NotFound(String),
    #[error("credentials request failed: {0}")]
    HttpError(String),
    #[error("INTEGRATIONS_SERVICE_URL not configured")]
    MissingConfig,
}

/// Fetch carrier credentials for a tenant from the Integrations module.
///
/// Calls `GET /api/integrations/internal/carrier-credentials/{connector_type}`
/// with the `X-App-Id` header set to the tenant's app_id.
///
/// Returns the connector config JSON blob on success, or a `CredentialsError`
/// if the config is missing or the request fails.
pub async fn get_carrier_credentials(
    client: &Client,
    app_id: &str,
    connector_type: &str,
) -> Result<Value, CredentialsError> {
    let base_url = std::env::var("INTEGRATIONS_SERVICE_URL")
        .map_err(|_| CredentialsError::MissingConfig)?;

    let url = format!(
        "{}/api/integrations/internal/carrier-credentials/{}",
        base_url.trim_end_matches('/'),
        connector_type
    );

    let resp = client
        .get(&url)
        .header("X-App-Id", app_id)
        .send()
        .await
        .map_err(|e| CredentialsError::HttpError(e.to_string()))?;

    match resp.status().as_u16() {
        200 => {
            let body: Value = resp
                .json()
                .await
                .map_err(|e| CredentialsError::HttpError(e.to_string()))?;
            Ok(body)
        }
        404 => Err(CredentialsError::NotFound(connector_type.to_string())),
        status => Err(CredentialsError::HttpError(format!(
            "unexpected status {} from integrations service",
            status
        ))),
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use serial_test::serial;

    /// Save and restore INTEGRATIONS_SERVICE_URL around a test body.
    macro_rules! with_integrations_url {
        ($value:expr, $body:expr) => {{
            let prev = std::env::var("INTEGRATIONS_SERVICE_URL").ok();
            match $value {
                Some(v) => std::env::set_var("INTEGRATIONS_SERVICE_URL", v),
                None => std::env::remove_var("INTEGRATIONS_SERVICE_URL"),
            }
            let result = $body;
            match prev {
                Some(v) => std::env::set_var("INTEGRATIONS_SERVICE_URL", v),
                None => std::env::remove_var("INTEGRATIONS_SERVICE_URL"),
            }
            result
        }};
    }

    /// When INTEGRATIONS_SERVICE_URL is absent, `MissingConfig` is returned.
    #[tokio::test]
    #[serial]
    async fn missing_config_when_env_unset() {
        let client = Client::new();
        let result =
            with_integrations_url!(None::<&str>, get_carrier_credentials(&client, "tenant-test", "stub").await);

        assert!(
            matches!(result, Err(CredentialsError::MissingConfig)),
            "expected MissingConfig when INTEGRATIONS_SERVICE_URL is not set"
        );
    }

    /// When the integrations service is unreachable, `HttpError` is returned —
    /// the caller (`fetch_credentials_or_empty`) handles this gracefully.
    #[tokio::test]
    #[serial]
    async fn unreachable_service_returns_http_error() {
        let client = Client::new();
        // Port 19999 on loopback — nothing listening there.
        let result = with_integrations_url!(
            Some("http://127.0.0.1:19999"),
            get_carrier_credentials(&client, "tenant-test", "stub").await
        );

        assert!(
            matches!(result, Err(CredentialsError::HttpError(_))),
            "expected HttpError for unreachable service, got {:?}",
            result
        );
    }
}
