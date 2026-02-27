pub mod accounts;
pub mod balance_transaction;
pub mod charge;
pub mod checkout_session;
pub mod customer;
pub mod dispute;
pub mod error;
pub mod events;
pub mod files;
pub mod payment_intent;
pub mod payment_method;
pub mod payouts;
pub mod refund;
pub mod subscription;
pub mod types;
pub mod users;
pub mod webhook;
pub mod webhook_endpoint;

use error::TilledError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for Tilled API client
#[derive(Debug, Clone)]
pub struct TilledConfig {
    pub secret_key: String,
    pub account_id: String,
    pub webhook_secret: String,
    pub sandbox: bool,
    pub base_path: String,
}

impl TilledConfig {
    /// Load configuration from environment variables for a specific app
    pub fn from_env(app_id: &str) -> Result<Self, TilledError> {
        let secret_key = get_var_with_app_fallback("TILLED_SECRET_KEY", app_id)?;
        let account_id = get_var_with_app_fallback("TILLED_ACCOUNT_ID", app_id)?;
        let webhook_secret = get_var_with_app_fallback("TILLED_WEBHOOK_SECRET", app_id)?;

        let sandbox = std::env::var("TILLED_SANDBOX")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .unwrap_or(false);

        let base_path = if sandbox {
            "https://sandbox-api.tilled.com".to_string()
        } else {
            "https://api.tilled.com".to_string()
        };

        Ok(TilledConfig {
            secret_key,
            account_id,
            webhook_secret,
            sandbox,
            base_path,
        })
    }
}

fn get_var_with_app_fallback(base_key: &str, app_id: &str) -> Result<String, TilledError> {
    let suffix = app_id.to_uppercase();
    let scoped_key = format!("{base_key}_{suffix}");
    if let Ok(value) = std::env::var(&scoped_key) {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }

    std::env::var(base_key).map_err(|_| {
        TilledError::ConfigError(format!("Missing {scoped_key} and fallback {base_key}"))
    })
}

/// Main Tilled API client
#[derive(Clone)]
pub struct TilledClient {
    config: Arc<TilledConfig>,
    http_client: Client,
}

impl TilledClient {
    /// Create a new Tilled client with the given configuration
    pub fn new(config: TilledConfig) -> Result<Self, TilledError> {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        Ok(TilledClient {
            config: Arc::new(config),
            http_client,
        })
    }

    /// Create a new Tilled client from environment variables
    pub fn from_env(app_id: &str) -> Result<Self, TilledError> {
        let config = TilledConfig::from_env(app_id)?;
        Self::new(config)
    }

    /// Make a GET request to the Tilled API
    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: Option<HashMap<String, String>>,
    ) -> Result<T, TilledError> {
        let url = format!("{}{}", self.config.base_path, path);
        let mut request = self
            .http_client
            .get(&url)
            .headers(self.build_auth_headers()?);

        if let Some(params) = params {
            request = request.query(&params);
        }

        let response = request
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a POST request to the Tilled API
    pub(crate) async fn post<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, TilledError> {
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .post(&url)
            .headers(self.build_auth_headers()?)
            .json(body)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a PATCH request to the Tilled API
    pub(crate) async fn patch<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, TilledError> {
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .patch(&url)
            .headers(self.build_auth_headers()?)
            .json(body)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a DELETE request to the Tilled API
    pub(crate) async fn delete<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, TilledError> {
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .delete(&url)
            .headers(self.build_auth_headers()?)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Handle HTTP response and convert to appropriate type or error
    async fn handle_response<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, TilledError> {
        let status = response.status();

        if status.is_success() {
            response
                .json::<T>()
                .await
                .map_err(|e| TilledError::ParseError(e.to_string()))
        } else {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());

            Err(TilledError::ApiError {
                status_code: status.as_u16(),
                message: error_body,
            })
        }
    }

    /// Get the config for webhook verification
    pub fn config(&self) -> &TilledConfig {
        &self.config
    }

    fn build_auth_headers(&self) -> Result<HeaderMap, TilledError> {
        build_auth_headers(&self.config.secret_key, &self.config.account_id)
    }
}

fn build_auth_headers(secret_key: &str, account_id: &str) -> Result<HeaderMap, TilledError> {
    let mut headers = HeaderMap::new();
    let auth = format!("Bearer {secret_key}");
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth).map_err(|e| TilledError::ValidationError(e.to_string()))?,
    );
    headers.insert(
        HeaderName::from_static("tilled-account"),
        HeaderValue::from_str(account_id)
            .map_err(|e| TilledError::ValidationError(e.to_string()))?,
    );
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::{build_auth_headers, TilledConfig};
    use reqwest::header::{HeaderName, AUTHORIZATION};
    use serial_test::serial;

    const APP_ID: &str = "ops";

    fn clear_keys() {
        let keys = [
            "TILLED_SECRET_KEY",
            "TILLED_ACCOUNT_ID",
            "TILLED_WEBHOOK_SECRET",
            "TILLED_SECRET_KEY_OPS",
            "TILLED_ACCOUNT_ID_OPS",
            "TILLED_WEBHOOK_SECRET_OPS",
            "TILLED_SANDBOX",
        ];
        for key in keys {
            std::env::remove_var(key);
        }
    }

    #[test]
    #[serial]
    fn config_prefers_app_scoped_credentials_over_global() {
        clear_keys();
        std::env::set_var("TILLED_SECRET_KEY", "global_sk");
        std::env::set_var("TILLED_ACCOUNT_ID", "global_acct");
        std::env::set_var("TILLED_WEBHOOK_SECRET", "global_wh");
        std::env::set_var("TILLED_SECRET_KEY_OPS", "scoped_sk");
        std::env::set_var("TILLED_ACCOUNT_ID_OPS", "scoped_acct");
        std::env::set_var("TILLED_WEBHOOK_SECRET_OPS", "scoped_wh");
        std::env::set_var("TILLED_SANDBOX", "true");

        let cfg = TilledConfig::from_env(APP_ID).unwrap();
        assert_eq!(cfg.secret_key, "scoped_sk");
        assert_eq!(cfg.account_id, "scoped_acct");
        assert_eq!(cfg.webhook_secret, "scoped_wh");
        assert!(cfg.sandbox);
        assert_eq!(cfg.base_path, "https://sandbox-api.tilled.com");
    }

    #[test]
    #[serial]
    fn config_falls_back_to_global_credentials_when_scoped_missing() {
        clear_keys();
        std::env::set_var("TILLED_SECRET_KEY", "global_sk");
        std::env::set_var("TILLED_ACCOUNT_ID", "global_acct");
        std::env::set_var("TILLED_WEBHOOK_SECRET", "global_wh");

        let cfg = TilledConfig::from_env(APP_ID).unwrap();
        assert_eq!(cfg.secret_key, "global_sk");
        assert_eq!(cfg.account_id, "global_acct");
        assert_eq!(cfg.webhook_secret, "global_wh");
    }

    #[test]
    fn auth_headers_include_expected_names_and_values() {
        let headers = build_auth_headers("tsk_123", "acct_123").unwrap();
        assert_eq!(headers[AUTHORIZATION], "Bearer tsk_123");
        assert_eq!(
            headers[HeaderName::from_static("tilled-account")],
            "acct_123"
        );
    }
}
