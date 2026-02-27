use super::error::TilledError;
use super::types::{BalanceTransaction, ListResponse};
use super::TilledClient;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct BalanceTransactionSummary {
    #[serde(flatten)]
    pub data: serde_json::Value,
}

impl TilledClient {
    /// List balance transactions in merchant account scope.
    pub async fn list_balance_transactions(
        &self,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<ListResponse<BalanceTransaction>, TilledError> {
        let params = build_pagination_params(offset, limit);
        self.get("/v1/balance-transactions", params).await
    }

    /// Get a balance transaction by ID.
    pub async fn get_balance_transaction(
        &self,
        id: &str,
    ) -> Result<BalanceTransaction, TilledError> {
        let path = format!("/v1/balance-transactions/{id}");
        self.get(&path, None).await
    }

    /// Get balance transaction summary with optional filters.
    pub async fn get_balance_transaction_summary(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<BalanceTransactionSummary, TilledError> {
        self.get("/v1/balance-transactions/summary", filters).await
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
    use serde_json::json;

    #[test]
    fn pagination_params_none_when_absent() {
        assert!(build_pagination_params(None, None).is_none());
    }

    #[test]
    fn pagination_params_include_offset_and_limit() {
        let params = build_pagination_params(Some(5), Some(15)).unwrap();
        assert_eq!(params.get("offset").map(String::as_str), Some("5"));
        assert_eq!(params.get("limit").map(String::as_str), Some("15"));
    }

    #[test]
    fn summary_flattens_unknown_payload_fields() {
        let value = json!({
            "available": {"amount": 123},
            "pending": {"amount": 45}
        });
        let parsed: super::BalanceTransactionSummary = serde_json::from_value(value).unwrap();
        assert!(parsed.data.get("available").is_some());
        assert!(parsed.data.get("pending").is_some());
    }
}
