use super::error::TilledError;
use super::types::{Dispute, ListResponse};
use super::TilledClient;
use std::collections::HashMap;

impl TilledClient {
    /// Get a dispute by ID
    pub async fn get_dispute(&self, dispute_id: &str) -> Result<Dispute, TilledError> {
        let path = format!("/v1/disputes/{}", dispute_id);
        self.get(&path, None).await
    }

    /// List disputes with optional filters
    pub async fn list_disputes(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<Dispute>, TilledError> {
        self.get("/v1/disputes", filters).await
    }
}
