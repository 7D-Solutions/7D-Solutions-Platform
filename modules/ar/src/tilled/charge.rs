use super::error::TilledError;
use super::types::Charge;
use super::TilledClient;

impl TilledClient {
    /// Get a charge by ID.
    pub async fn get_charge(&self, charge_id: &str) -> Result<Charge, TilledError> {
        let path = format!("/v1/charges/{charge_id}");
        self.get(&path, None).await
    }
}
