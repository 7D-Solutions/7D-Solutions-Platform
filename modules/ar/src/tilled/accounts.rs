use super::error::TilledError;
use super::types::{Account, AccountCapability, ConnectedAccount, ListResponse, Metadata};
use super::TilledClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct CreateConnectedAccountRequest {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_accounts: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// Request body for updating the current account (PATCH /v1/accounts).
/// Uses `serde_json::Value` for metadata so that `null` values can be sent
/// to delete individual keys (Tilled merges metadata, so omitting a key doesn't remove it).
#[derive(Debug, Serialize, Clone)]
pub struct UpdateAccountRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Request body for adding a capability (POST /v1/accounts/capabilities).
#[derive(Debug, Serialize)]
pub struct AddCapabilityRequest {
    pub pricing_template_id: String,
}

/// Request body for updating a capability (POST /v1/accounts/capabilities/{id}).
#[derive(Debug, Serialize)]
pub struct UpdateCapabilityRequest {
    pub pricing_template_id: String,
}

impl TilledClient {
    /// List connected merchant accounts in partner scope.
    pub async fn list_connected_accounts(
        &self,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<ListResponse<ConnectedAccount>, TilledError> {
        let params = build_pagination_params(offset, limit);
        self.get("/v1/accounts/connected", params).await
    }

    /// Create a connected merchant account.
    pub async fn create_connected_account(
        &self,
        email: String,
        name: Option<String>,
        capabilities: Option<serde_json::Value>,
        bank_accounts: Option<Vec<serde_json::Value>>,
        metadata: Option<Metadata>,
    ) -> Result<ConnectedAccount, TilledError> {
        let request = CreateConnectedAccountRequest {
            email,
            name,
            capabilities,
            bank_accounts,
            metadata,
        };

        self.post("/v1/accounts/connected", &request).await
    }

    /// Get account details by account ID.
    pub async fn get_account(&self, account_id: &str) -> Result<ConnectedAccount, TilledError> {
        let path = format!("/v1/accounts/{account_id}");
        self.get(&path, None).await
    }

    /// Get the current account (self) based on the tilled-account header.
    pub async fn get_self_account(&self) -> Result<Account, TilledError> {
        self.get("/v1/accounts", None).await
    }

    /// Update the current account (self).
    pub async fn update_account(
        &self,
        request: &UpdateAccountRequest,
    ) -> Result<Account, TilledError> {
        self.patch("/v1/accounts", request).await
    }

    /// Add a capability to the current account.
    /// Requires `pricing_template_id`. May fail if onboarding is already submitted.
    pub async fn add_account_capability(
        &self,
        request: &AddCapabilityRequest,
    ) -> Result<AccountCapability, TilledError> {
        self.post("/v1/accounts/capabilities", request).await
    }

    /// Update an existing account capability.
    /// Tilled returns 201 with empty body on success.
    pub async fn update_account_capability(
        &self,
        capability_id: &str,
        request: &UpdateCapabilityRequest,
    ) -> Result<(), TilledError> {
        let path = format!("/v1/accounts/capabilities/{capability_id}");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .post(&url)
            .headers(self.build_auth_headers()?)
            .json(request)
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

    /// Delete an account capability.
    /// Tilled may return 204 with empty body. May fail if onboarding is already submitted.
    pub async fn delete_account_capability(&self, capability_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/accounts/capabilities/{capability_id}");
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

fn build_pagination_params(
    offset: Option<i64>,
    limit: Option<i64>,
) -> Option<HashMap<String, String>> {
    let mut params = HashMap::new();
    if let Some(offset) = offset {
        params.insert("offset".to_string(), offset.to_string());
    }
    if let Some(limit) = limit {
        params.insert("limit".to_string(), limit.to_string());
    }
    if params.is_empty() {
        None
    } else {
        Some(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_params_include_only_present_values() {
        let params = build_pagination_params(Some(10), None).expect("test fixture");
        assert_eq!(params.get("offset").map(String::as_str), Some("10"));
        assert!(!params.contains_key("limit"));
    }

    #[test]
    fn update_account_request_omits_none_fields() {
        let req = UpdateAccountRequest {
            metadata: None,
            name: Some("Test".to_string()),
            email: None,
        };
        let value = serde_json::to_value(&req).expect("test fixture");
        assert!(value.get("metadata").is_none());
        assert_eq!(value.get("name").expect("test fixture"), "Test");
        assert!(value.get("email").is_none());
    }

    #[test]
    fn update_account_request_supports_null_metadata_keys() {
        let req = UpdateAccountRequest {
            metadata: Some(serde_json::json!({"keep": "yes", "remove": null})),
            name: None,
            email: None,
        };
        let value = serde_json::to_value(&req).expect("test fixture");
        let meta = value.get("metadata").expect("test fixture");
        assert_eq!(meta.get("keep").expect("test fixture"), "yes");
        assert!(meta.get("remove").expect("test fixture").is_null());
    }

    #[test]
    fn add_capability_request_serializes() {
        let req = AddCapabilityRequest {
            pricing_template_id: "pt_123".to_string(),
        };
        let value = serde_json::to_value(req).expect("test fixture");
        assert_eq!(value.get("pricing_template_id").expect("test fixture"), "pt_123");
    }

    #[test]
    fn update_capability_request_serializes() {
        let req = UpdateCapabilityRequest {
            pricing_template_id: "pt_456".to_string(),
        };
        let value = serde_json::to_value(req).expect("test fixture");
        assert_eq!(value.get("pricing_template_id").expect("test fixture"), "pt_456");
    }
}
