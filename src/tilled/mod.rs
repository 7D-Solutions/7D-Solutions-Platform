pub mod customer;
pub mod payment_method;
pub mod payment_intent;
pub mod subscription;
pub mod refund;
pub mod dispute;
pub mod webhook;
pub mod error;
pub mod types;

use error::TilledError;
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
        let prefix = app_id.to_uppercase();
        let secret_key = std::env::var(format!("TILLED_SECRET_KEY_{}", prefix))
            .map_err(|_| TilledError::ConfigError(format!("Missing TILLED_SECRET_KEY_{}", prefix)))?;
        let account_id = std::env::var(format!("TILLED_ACCOUNT_ID_{}", prefix))
            .map_err(|_| TilledError::ConfigError(format!("Missing TILLED_ACCOUNT_ID_{}", prefix)))?;
        let webhook_secret = std::env::var(format!("TILLED_WEBHOOK_SECRET_{}", prefix))
            .map_err(|_| TilledError::ConfigError(format!("Missing TILLED_WEBHOOK_SECRET_{}", prefix)))?;

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
        let mut request = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.secret_key))
            .header("tilled-account", &self.config.account_id);

        if let Some(params) = params {
            request = request.query(&params);
        }

        let response = request.send().await
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
        let response = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.secret_key))
            .header("tilled-account", &self.config.account_id)
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
        let response = self.http_client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.config.secret_key))
            .header("tilled-account", &self.config.account_id)
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
        let response = self.http_client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.config.secret_key))
            .header("tilled-account", &self.config.account_id)
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
            response.json::<T>().await
                .map_err(|e| TilledError::ParseError(e.to_string()))
        } else {
            let error_body = response.text().await
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
}
