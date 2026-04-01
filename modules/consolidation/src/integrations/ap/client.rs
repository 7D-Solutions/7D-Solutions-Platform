//! AP HTTP client adapter — wraps platform-client-ap for payable data.
//!
//! Used by intercompany matching to identify payables where the
//! vendor is another entity in the consolidation group.

use platform_sdk::{ClientError, parse_response};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Summary of payable balance per vendor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayableSummary {
    pub vendor_id: String,
    pub vendor_name: String,
    pub currency: String,
    pub outstanding_minor: i64,
    pub bill_count: i64,
}

/// HTTP client adapter for the AP service.
///
/// Wraps `platform-client-ap` as the upstream dependency. Uses raw reqwest
/// with `parse_response` because the generated `ReportsClient::aging_report`
/// does not return a typed response body.
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
    ) -> Result<Vec<PayableSummary>, ClientError> {
        let url = format!("{}/api/ap/aging", self.base_url);
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
