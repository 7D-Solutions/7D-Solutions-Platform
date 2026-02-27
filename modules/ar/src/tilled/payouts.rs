use super::error::TilledError;
use super::types::{ListResponse, Payout};
use super::TilledClient;
use std::collections::HashMap;

impl TilledClient {
    /// List payouts in merchant account scope.
    pub async fn list_payouts(
        &self,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<ListResponse<Payout>, TilledError> {
        let params = build_pagination_params(offset, limit);
        self.get("/v1/payouts", params).await
    }

    /// Get payout details by payout ID.
    pub async fn get_payout(&self, payout_id: &str) -> Result<Payout, TilledError> {
        let path = format!("/v1/payouts/{payout_id}");
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
    fn pagination_params_include_offset_and_limit() {
        let params = build_pagination_params(Some(20), Some(50)).unwrap();
        assert_eq!(params.get("offset").map(String::as_str), Some("20"));
        assert_eq!(params.get("limit").map(String::as_str), Some("50"));
    }
}
