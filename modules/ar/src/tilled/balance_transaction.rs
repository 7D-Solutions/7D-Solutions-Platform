use super::error::TilledError;
use super::types::{BalanceTransaction, ListResponse};
use super::TilledClient;
use std::collections::HashMap;

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
    fn pagination_params_none_when_absent() {
        assert!(build_pagination_params(None, None).is_none());
    }
}
