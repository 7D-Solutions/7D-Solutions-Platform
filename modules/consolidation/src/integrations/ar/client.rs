//! AR HTTP client adapter — wraps platform-client-ar for receivable data.
//!
//! Used by intercompany matching to identify receivables where the
//! customer is another entity in the consolidation group.

use platform_sdk::{ClientError, parse_response};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Summary of receivable balance per customer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivableSummary {
    pub customer_id: String,
    pub customer_name: String,
    pub currency: String,
    pub outstanding_minor: i64,
    pub invoice_count: i64,
}

/// HTTP client adapter for the AR service.
///
/// Wraps `platform-client-ar` as the upstream dependency. Uses raw reqwest
/// with `parse_response` because the generated `AgingClient::get_aging`
/// does not return a typed response body.
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
    ) -> Result<Vec<ReceivableSummary>, ClientError> {
        let url = format!("{}/api/ar/aging", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("tenant_id", tenant_id)])
            .send()
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
