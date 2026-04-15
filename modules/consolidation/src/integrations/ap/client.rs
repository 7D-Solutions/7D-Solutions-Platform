//! AP HTTP client adapter — wraps platform-client-ap for payable data.
//!
//! Used by intercompany matching to identify payables where the
//! vendor is another entity in the consolidation group.

use platform_sdk::{parse_response, ClientError, PlatformClient};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
/// Uses `PlatformClient` from `platform-sdk` for tenant header injection,
/// correlation IDs, and automatic retry on 429/503 for GET requests.
pub struct ApClient {
    client: PlatformClient,
}

impl ApClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: PlatformClient::new(base_url.trim_end_matches('/').to_string()),
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
        let tenant_uuid = Uuid::parse_str(tenant_id).map_err(|e| ClientError::Unexpected {
            status: 0,
            body: format!("invalid tenant_id: {e}"),
        })?;
        let claims = PlatformClient::service_claims(tenant_uuid);
        let resp = self
            .client
            .get("/api/ap/aging", &claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
