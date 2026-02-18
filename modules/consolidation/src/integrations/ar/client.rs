//! AR HTTP client — fetches receivable data from the AR service.
//!
//! Used by intercompany matching to identify receivables where the
//! customer is another entity in the consolidation group.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArClientError {
    #[error("AR API request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("AR API returned {status}: {body}")]
    Api { status: u16, body: String },
}

/// Summary of receivable balance per customer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivableSummary {
    pub customer_id: String,
    pub customer_name: String,
    pub currency: String,
    pub outstanding_minor: i64,
    pub invoice_count: i64,
}

/// HTTP client for the AR service.
#[derive(Clone)]
pub struct ArClient {
    client: Client,
    base_url: String,
}

impl ArClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch receivable summaries grouped by customer for a tenant.
    ///
    /// Returns balances that can be matched against AP payables from
    /// counterparty entities in the consolidation group.
    pub async fn get_receivable_summaries(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<ReceivableSummary>, ArClientError> {
        let url = format!("{}/api/ar/aging", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("tenant_id", tenant_id)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ArClientError::Api { status, body });
        }

        Ok(resp.json().await?)
    }
}
