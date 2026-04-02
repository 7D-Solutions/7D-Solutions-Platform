//! AR HTTP client adapter — wraps platform-client-ar for receivable data.
//!
//! Used by intercompany matching to identify receivables where the
//! customer is another entity in the consolidation group.

use platform_sdk::{ClientError, PlatformClient, parse_response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
/// Uses `PlatformClient` from `platform-sdk` for tenant header injection,
/// correlation IDs, and automatic retry on 429/503 for GET requests.
pub struct ArClient {
    client: PlatformClient,
}

impl ArClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: PlatformClient::new(
                base_url.trim_end_matches('/').to_string(),
            ),
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
        let tenant_uuid = Uuid::parse_str(tenant_id).map_err(|e| {
            ClientError::Unexpected { status: 0, body: format!("invalid tenant_id: {e}") }
        })?;
        let claims = PlatformClient::service_claims(tenant_uuid);
        let resp = self.client.get("/api/ar/aging", &claims).await.map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
