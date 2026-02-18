//! AP HTTP client — fetches payable data from the AP service.
//!
//! Used by intercompany matching to identify payables where the
//! vendor is another entity in the consolidation group.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApClientError {
    #[error("AP API request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("AP API returned {status}: {body}")]
    Api { status: u16, body: String },
}

/// Summary of payable balance per vendor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayableSummary {
    pub vendor_id: String,
    pub vendor_name: String,
    pub currency: String,
    pub outstanding_minor: i64,
    pub bill_count: i64,
}

/// HTTP client for the AP service.
#[derive(Clone)]
pub struct ApClient {
    client: Client,
    base_url: String,
}

impl ApClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch payable summaries grouped by vendor for a tenant.
    ///
    /// Returns balances that can be matched against AR receivables from
    /// counterparty entities in the consolidation group.
    pub async fn get_payable_summaries(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<PayableSummary>, ApClientError> {
        let url = format!("{}/api/ap/aging", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("tenant_id", tenant_id)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApClientError::Api { status, body });
        }

        Ok(resp.json().await?)
    }
}
