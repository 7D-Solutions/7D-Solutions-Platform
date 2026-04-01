//! Integration client for the Inventory module.
//!
//! Exactly-once inventory movements for shipment lifecycle actions:
//! - Inbound close → receipt per accepted line (qty_accepted > 0)
//! - Outbound ship → issue per shipped line (qty_shipped > 0)
//!
//! Idempotency key: `sr:{tenant_id}:{shipment_id}:{line_id}:{action}`
//! Line-level guard: `inventory_ref_id IS NOT NULL` → skip.
//!
//! Two modes:
//! - `Http` — calls the Inventory module via typed `platform-client-inventory`
//! - `Deterministic` — derives stable UUIDs from idempotency keys
//!   (same pattern as AP → Payments: `modules/ap/src/integrations/payments/`)

use platform_client_inventory::{
    IssueRequest, IssuesClient, ReceiptRequest, ReceiptsClient,
};
use platform_sdk::ClientError;
use thiserror::Error;
use uuid::Uuid;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("Inventory client error: {0}")]
    Client(#[from] ClientError),
}

// ── Client ───────────────────────────────────────────────────

/// Inventory integration client.
///
/// `Http` makes real HTTP calls to the Inventory module's receipt/issue
/// endpoints via typed `platform-client-inventory` clients.
/// `Deterministic` derives stable IDs from business keys using UUID v5 —
/// useful during early integration before the Inventory HTTP service is
/// available.
#[derive(Debug, Clone)]
pub struct InventoryIntegration {
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    Http {
        base_url: String,
        client: reqwest::Client,
        token: String,
    },
    Deterministic,
}

impl InventoryIntegration {
    /// Create an HTTP-backed client pointing at the Inventory module.
    pub fn http(base_url: &str, token: &str) -> Self {
        Self {
            mode: Mode::Http {
                base_url: base_url.trim_end_matches('/').to_string(),
                client: reqwest::Client::new(),
                token: token.to_string(),
            },
        }
    }

    /// Create a deterministic client that derives IDs from idempotency keys.
    /// No external calls are made — IDs are stable UUID v5 hashes.
    pub fn deterministic() -> Self {
        Self {
            mode: Mode::Deterministic,
        }
    }

    /// Create an inventory receipt for an accepted inbound line.
    /// Returns the inventory reference ID (receipt_line_id).
    pub async fn create_receipt(
        &self,
        tenant_id: Uuid,
        shipment_id: Uuid,
        line_id: Uuid,
        warehouse_id: Uuid,
        quantity: i64,
        currency: &str,
    ) -> Result<Uuid, InventoryError> {
        let idem_key = make_idempotency_key(tenant_id, shipment_id, line_id, "receipt");

        match &self.mode {
            Mode::Http { base_url, client, token } => {
                let item_id = derive_id(&format!("item:{}:{}", tenant_id, idem_key));
                let receipts = ReceiptsClient::new(client.clone(), base_url, token);
                let body = ReceiptRequest {
                    tenant_id: tenant_id.to_string(),
                    item_id,
                    warehouse_id,
                    quantity,
                    unit_cost_minor: 1, // placeholder — cost reconciliation via AP/PO
                    currency: currency.to_string(),
                    idempotency_key: idem_key,
                    causation_id: None,
                    correlation_id: None,
                    location_id: None,
                    lot_code: None,
                    purchase_order_id: None,
                    serial_codes: None,
                    source_type: None,
                    uom_id: None,
                };
                let result = receipts.post_receipt(&body).await?;
                Ok(result.receipt_line_id)
            }
            Mode::Deterministic => Ok(derive_id(&idem_key)),
        }
    }

    /// Create an inventory issue for a shipped outbound line.
    /// Returns the inventory reference ID (issue_line_id).
    pub async fn create_issue(
        &self,
        tenant_id: Uuid,
        shipment_id: Uuid,
        line_id: Uuid,
        warehouse_id: Uuid,
        quantity: i64,
        currency: &str,
    ) -> Result<Uuid, InventoryError> {
        let idem_key = make_idempotency_key(tenant_id, shipment_id, line_id, "issue");

        match &self.mode {
            Mode::Http { base_url, client, token } => {
                let item_id = derive_id(&format!("item:{}:{}", tenant_id, idem_key));
                let issues = IssuesClient::new(client.clone(), base_url, token);
                let body = IssueRequest {
                    tenant_id: tenant_id.to_string(),
                    item_id,
                    warehouse_id,
                    quantity,
                    currency: currency.to_string(),
                    source_module: "shipping-receiving".to_string(),
                    source_type: "shipment".to_string(),
                    source_id: shipment_id.to_string(),
                    source_line_id: Some(line_id.to_string()),
                    idempotency_key: idem_key,
                    causation_id: None,
                    correlation_id: None,
                    location_id: None,
                    lot_code: None,
                    serial_codes: None,
                    uom_id: None,
                };
                let result = issues.post_issue(&body).await?;
                Ok(result.issue_line_id)
            }
            Mode::Deterministic => Ok(derive_id(&idem_key)),
        }
    }
}

// ── Idempotency key ──────────────────────────────────────────

/// Build a deterministic idempotency key from business identifiers.
///
/// Format: `sr:{tenant_id}:{shipment_id}:{line_id}:{action}`
/// where action is "receipt" or "issue".
pub fn make_idempotency_key(
    tenant_id: Uuid,
    shipment_id: Uuid,
    line_id: Uuid,
    action: &str,
) -> String {
    format!("sr:{}:{}:{}:{}", tenant_id, shipment_id, line_id, action)
}

/// Derive a deterministic UUID from an idempotency key using UUID v5.
fn derive_id(idempotency_key: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, idempotency_key.as_bytes())
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_is_deterministic() {
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let k1 = make_idempotency_key(t, s, l, "receipt");
        let k2 = make_idempotency_key(t, s, l, "receipt");
        assert_eq!(k1, k2);
    }

    #[test]
    fn idempotency_key_differs_by_action() {
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let k1 = make_idempotency_key(t, s, l, "receipt");
        let k2 = make_idempotency_key(t, s, l, "issue");
        assert_ne!(k1, k2);
    }

    #[test]
    fn derive_id_is_stable() {
        let id1 = derive_id("sr:t1:s1:l1:receipt");
        let id2 = derive_id("sr:t1:s1:l1:receipt");
        assert_eq!(id1, id2);
    }

    #[test]
    fn derive_id_differs_for_different_keys() {
        let id1 = derive_id("sr:t1:s1:l1:receipt");
        let id2 = derive_id("sr:t1:s1:l2:receipt");
        assert_ne!(id1, id2);
    }

    #[tokio::test]
    async fn deterministic_client_creates_stable_receipt() -> Result<(), InventoryError> {
        let client = InventoryIntegration::deterministic();
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let w = Uuid::new_v4();

        let id1 = client.create_receipt(t, s, l, w, 10, "usd").await?;
        let id2 = client.create_receipt(t, s, l, w, 10, "usd").await?;
        assert_eq!(id1, id2, "same inputs must produce same receipt ID");
        Ok(())
    }

    #[tokio::test]
    async fn deterministic_client_creates_stable_issue() -> Result<(), InventoryError> {
        let client = InventoryIntegration::deterministic();
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let w = Uuid::new_v4();

        let id1 = client.create_issue(t, s, l, w, 5, "usd").await?;
        let id2 = client.create_issue(t, s, l, w, 5, "usd").await?;
        assert_eq!(id1, id2, "same inputs must produce same issue ID");
        Ok(())
    }

    #[tokio::test]
    async fn receipt_and_issue_ids_differ_for_same_line() -> Result<(), InventoryError> {
        let client = InventoryIntegration::deterministic();
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let w = Uuid::new_v4();

        let receipt = client.create_receipt(t, s, l, w, 10, "usd").await?;
        let issue = client.create_issue(t, s, l, w, 10, "usd").await?;
        assert_ne!(receipt, issue, "receipt and issue must have different IDs");
        Ok(())
    }
}
