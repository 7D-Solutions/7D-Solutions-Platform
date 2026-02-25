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
//! - `Http` — calls the Inventory module REST API (production)
//! - `Deterministic` — derives stable UUIDs from idempotency keys
//!   (same pattern as AP → Payments: `modules/ap/src/integrations/payments/`)

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("HTTP request failed: {0}")]
    Http(String),

    #[error("Inventory API returned {status}: {body}")]
    Api { status: u16, body: String },
}

// ── Client ───────────────────────────────────────────────────

/// Inventory integration client.
///
/// `Http` makes real HTTP calls to the Inventory module's receipt/issue
/// endpoints. `Deterministic` derives stable IDs from business keys using
/// UUID v5 — useful during early integration before the Inventory HTTP
/// service is available.
#[derive(Debug, Clone)]
pub struct InventoryIntegration {
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    Http {
        base_url: String,
        client: reqwest::Client,
    },
    Deterministic,
}

impl InventoryIntegration {
    /// Create an HTTP-backed client pointing at the Inventory module.
    pub fn http(base_url: &str) -> Self {
        Self {
            mode: Mode::Http {
                base_url: base_url.trim_end_matches('/').to_string(),
                client: reqwest::Client::new(),
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
            Mode::Http { base_url, client } => {
                create_receipt_http(
                    client,
                    base_url,
                    &tenant_id.to_string(),
                    warehouse_id,
                    quantity,
                    currency,
                    &idem_key,
                )
                .await
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
            Mode::Http { base_url, client } => {
                create_issue_http(
                    client,
                    base_url,
                    &tenant_id.to_string(),
                    shipment_id,
                    line_id,
                    warehouse_id,
                    quantity,
                    currency,
                    &idem_key,
                )
                .await
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

// ── HTTP request/response types ──────────────────────────────

#[derive(Debug, Serialize)]
struct ReceiptHttpReq {
    tenant_id: String,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    unit_cost_minor: i64,
    currency: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
struct ReceiptHttpResp {
    receipt_line_id: Uuid,
}

#[derive(Debug, Serialize)]
struct IssueHttpReq {
    tenant_id: String,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    currency: String,
    source_module: String,
    source_type: String,
    source_id: String,
    source_line_id: Option<String>,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
struct IssueHttpResp {
    issue_line_id: Uuid,
}

// ── HTTP helpers ─────────────────────────────────────────────

/// POST /api/inventory/receipts
///
/// NOTE: The Inventory receipt API requires `item_id` which is not yet
/// available on shipment lines (they carry SKU only). Until a SKU → item_id
/// resolution endpoint exists, `item_id` is derived deterministically from
/// the idempotency key. This means HTTP mode currently requires pre-seeded
/// inventory items matching these derived IDs.
async fn create_receipt_http(
    client: &reqwest::Client,
    base_url: &str,
    tenant_id: &str,
    warehouse_id: Uuid,
    quantity: i64,
    currency: &str,
    idempotency_key: &str,
) -> Result<Uuid, InventoryError> {
    let item_id = derive_id(&format!("item:{}:{}", tenant_id, idempotency_key));

    let url = format!("{}/api/inventory/receipts", base_url);
    let body = ReceiptHttpReq {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity,
        unit_cost_minor: 1, // placeholder — cost reconciliation via AP/PO
        currency: currency.to_string(),
        idempotency_key: idempotency_key.to_string(),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| InventoryError::Http(e.to_string()))?;

    let status = resp.status().as_u16();
    if status == 200 || status == 201 {
        let result: ReceiptHttpResp = resp
            .json()
            .await
            .map_err(|e| InventoryError::Http(e.to_string()))?;
        Ok(result.receipt_line_id)
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(InventoryError::Api { status, body })
    }
}

/// POST /api/inventory/issues
async fn create_issue_http(
    client: &reqwest::Client,
    base_url: &str,
    tenant_id: &str,
    shipment_id: Uuid,
    line_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    currency: &str,
    idempotency_key: &str,
) -> Result<Uuid, InventoryError> {
    let item_id = derive_id(&format!("item:{}:{}", tenant_id, idempotency_key));

    let url = format!("{}/api/inventory/issues", base_url);
    let body = IssueHttpReq {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity,
        currency: currency.to_string(),
        source_module: "shipping-receiving".to_string(),
        source_type: "shipment".to_string(),
        source_id: shipment_id.to_string(),
        source_line_id: Some(line_id.to_string()),
        idempotency_key: idempotency_key.to_string(),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| InventoryError::Http(e.to_string()))?;

    let status = resp.status().as_u16();
    if status == 200 || status == 201 {
        let result: IssueHttpResp = resp
            .json()
            .await
            .map_err(|e| InventoryError::Http(e.to_string()))?;
        Ok(result.issue_line_id)
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(InventoryError::Api { status, body })
    }
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
    async fn deterministic_client_creates_stable_receipt() {
        let client = InventoryIntegration::deterministic();
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let w = Uuid::new_v4();

        let id1 = client.create_receipt(t, s, l, w, 10, "usd").await.unwrap();
        let id2 = client.create_receipt(t, s, l, w, 10, "usd").await.unwrap();
        assert_eq!(id1, id2, "same inputs must produce same receipt ID");
    }

    #[tokio::test]
    async fn deterministic_client_creates_stable_issue() {
        let client = InventoryIntegration::deterministic();
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let w = Uuid::new_v4();

        let id1 = client.create_issue(t, s, l, w, 5, "usd").await.unwrap();
        let id2 = client.create_issue(t, s, l, w, 5, "usd").await.unwrap();
        assert_eq!(id1, id2, "same inputs must produce same issue ID");
    }

    #[tokio::test]
    async fn receipt_and_issue_ids_differ_for_same_line() {
        let client = InventoryIntegration::deterministic();
        let t = Uuid::new_v4();
        let s = Uuid::new_v4();
        let l = Uuid::new_v4();
        let w = Uuid::new_v4();

        let receipt = client.create_receipt(t, s, l, w, 10, "usd").await.unwrap();
        let issue = client.create_issue(t, s, l, w, 10, "usd").await.unwrap();
        assert_ne!(receipt, issue, "receipt and issue must have different IDs");
    }
}
