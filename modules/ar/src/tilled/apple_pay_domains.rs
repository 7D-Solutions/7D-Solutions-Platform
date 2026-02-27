use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::{Deserialize, Serialize};

/// Apple Pay domain registration from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct ApplePayDomain {
    pub id: String,
    #[serde(default)]
    pub domain_name: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Request body for creating an Apple Pay domain.
#[derive(Debug, Serialize)]
pub struct CreateApplePayDomainRequest {
    pub domain_name: String,
}

impl TilledClient {
    /// Register an Apple Pay domain.
    pub async fn create_apple_pay_domain(
        &self,
        domain_name: String,
    ) -> Result<ApplePayDomain, TilledError> {
        let request = CreateApplePayDomainRequest { domain_name };
        self.post("/v1/apple-pay-domains", &request).await
    }

    /// List registered Apple Pay domains.
    pub async fn list_apple_pay_domains(
        &self,
    ) -> Result<ListResponse<ApplePayDomain>, TilledError> {
        self.get("/v1/apple-pay-domains", None).await
    }

    /// Get an Apple Pay domain by ID.
    pub async fn get_apple_pay_domain(
        &self,
        domain_id: &str,
    ) -> Result<ApplePayDomain, TilledError> {
        let path = format!("/v1/apple-pay-domains/{domain_id}");
        self.get(&path, None).await
    }

    /// Delete an Apple Pay domain registration.
    /// Tilled may return 204 with empty body for successful deletes.
    pub async fn delete_apple_pay_domain(&self, domain_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/apple-pay-domains/{domain_id}");
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
    fn apple_pay_domain_deserializes_full() {
        let value = serde_json::json!({
            "id": "apd_123",
            "domain_name": "pay.example.com",
            "account_id": "acct_456",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let domain: ApplePayDomain = serde_json::from_value(value).unwrap();
        assert_eq!(domain.id, "apd_123");
        assert_eq!(domain.domain_name.as_deref(), Some("pay.example.com"));
    }

    #[test]
    fn apple_pay_domain_deserializes_minimal() {
        let value = serde_json::json!({"id": "apd_min"});
        let domain: ApplePayDomain = serde_json::from_value(value).unwrap();
        assert_eq!(domain.id, "apd_min");
        assert!(domain.domain_name.is_none());
    }

    #[test]
    fn create_request_serializes() {
        let req = CreateApplePayDomainRequest {
            domain_name: "shop.example.com".to_string(),
        };
        let value = serde_json::to_value(req).unwrap();
        assert_eq!(value.get("domain_name").unwrap(), "shop.example.com");
    }
}
