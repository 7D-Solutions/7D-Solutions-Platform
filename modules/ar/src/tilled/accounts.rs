use super::error::TilledError;
use super::types::{ConnectedAccount, ListResponse, Metadata};
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
    use super::build_pagination_params;

    #[test]
    fn pagination_params_include_only_present_values() {
        let params = build_pagination_params(Some(10), None).unwrap();
        assert_eq!(params.get("offset").map(String::as_str), Some("10"));
        assert!(!params.contains_key("limit"));
    }
}
