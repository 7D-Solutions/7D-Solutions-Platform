//! Dataset digest computation for demo-seed
//!
//! Computes a deterministic SHA256 hash of the created resource set.
//! The digest is used to verify that two runs with the same seed
//! produce identical datasets.
//!
//! **Algorithm:**
//! 1. Collect all (resource_type, correlation_id, value) tuples
//! 2. Sort by (resource_type, correlation_id) for determinism
//! 3. SHA256 the canonical JSON representation

use sha2::{Digest, Sha256};

/// A single recorded resource entry
#[derive(Debug, Clone)]
struct ResourceEntry {
    resource_type: &'static str,
    correlation_id: String,
    /// Canonical value (e.g. DB ID, amount_cents) as string
    value: String,
}

/// Tracks created resources and computes a final digest
#[derive(Debug, Default)]
pub struct DigestTracker {
    entries: Vec<ResourceEntry>,
}

impl DigestTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a created customer
    pub fn record_customer(&mut self, customer_id: i32, correlation_id: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "customer",
            correlation_id: correlation_id.to_string(),
            value: customer_id.to_string(),
        });
    }

    /// Record a created numbering policy
    pub fn record_numbering_policy(&mut self, entity: &str, prefix: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "numbering_policy",
            correlation_id: entity.to_string(),
            value: prefix.to_string(),
        });
    }

    /// Record a created GL account
    pub fn record_gl_account(&mut self, code: &str, name: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "gl_account",
            correlation_id: code.to_string(),
            value: name.to_string(),
        });
    }

    /// Record a created FX rate
    pub fn record_fx_rate(&mut self, rate_id: uuid::Uuid, pair: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "fx_rate",
            correlation_id: pair.to_string(),
            value: rate_id.to_string(),
        });
    }

    /// Record a created inventory item
    pub fn record_item(&mut self, item_id: uuid::Uuid, sku: &str, make_buy: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "item",
            correlation_id: sku.to_string(),
            value: format!("{}/{}", item_id, make_buy),
        });
    }

    /// Record a created warehouse location
    pub fn record_location(&mut self, location_id: uuid::Uuid, code: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "location",
            correlation_id: code.to_string(),
            value: location_id.to_string(),
        });
    }

    /// Record a created unit of measure
    pub fn record_uom(&mut self, uom_id: uuid::Uuid, code: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "uom",
            correlation_id: code.to_string(),
            value: uom_id.to_string(),
        });
    }

    /// Record a created work center
    pub fn record_workcenter(&mut self, workcenter_id: uuid::Uuid, code: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "workcenter",
            correlation_id: code.to_string(),
            value: workcenter_id.to_string(),
        });
    }

    /// Record a created routing
    pub fn record_routing(&mut self, routing_id: uuid::Uuid, item_id: uuid::Uuid, revision: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "routing",
            correlation_id: format!("{}/{}", item_id, revision),
            value: routing_id.to_string(),
        });
    }

    /// Record a created BOM header
    pub fn record_bom(&mut self, bom_id: uuid::Uuid, part_id: uuid::Uuid) {
        self.entries.push(ResourceEntry {
            resource_type: "bom",
            correlation_id: part_id.to_string(),
            value: bom_id.to_string(),
        });
    }

    /// Record a created BOM line
    pub fn record_bom_line(
        &mut self,
        line_id: uuid::Uuid,
        component_item_id: uuid::Uuid,
        quantity: f64,
    ) {
        self.entries.push(ResourceEntry {
            resource_type: "bom_line",
            correlation_id: component_item_id.to_string(),
            value: format!("{}/{}", line_id, quantity),
        });
    }

    /// Record a created party (customer or supplier)
    pub fn record_party(&mut self, party_id: uuid::Uuid, name: &str, role: &str) {
        self.entries.push(ResourceEntry {
            resource_type: "party",
            correlation_id: format!("{}/{}", role, name),
            value: party_id.to_string(),
        });
    }

    /// Record a created invoice with its amount
    pub fn record_invoice(&mut self, invoice_id: i32, correlation_id: &str, amount_cents: i32) {
        self.entries.push(ResourceEntry {
            resource_type: "invoice",
            correlation_id: correlation_id.to_string(),
            value: format!("{}/{}", invoice_id, amount_cents),
        });
    }

    /// Finalize and compute the digest.
    ///
    /// Returns a hex-encoded SHA256 hash of the sorted resource list.
    pub fn finalize(mut self) -> String {
        // Sort for determinism regardless of creation order
        self.entries.sort_by(|a, b| {
            a.resource_type
                .cmp(b.resource_type)
                .then(a.correlation_id.cmp(&b.correlation_id))
        });

        let canonical: Vec<serde_json::Value> = self
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "type": e.resource_type,
                    "correlation_id": e.correlation_id,
                    "value": e.value,
                })
            })
            .collect();

        let json = serde_json::to_string(&canonical).expect("Failed to serialize entries");
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// Compute the expected digest without making any HTTP calls.
///
/// This relies only on the seed, tenant, and count parameters —
/// it does NOT know the actual DB IDs, so it hashes the correlation IDs
/// and amounts directly. This gives a stable "configuration hash" that
/// verifies the same seed produces the same input parameters.
pub fn expected_digest(
    tenant: &str,
    seed: u64,
    customers: usize,
    invoices_per_customer: usize,
) -> String {
    use rand::Rng;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut entries: Vec<serde_json::Value> = Vec::new();

    for customer_idx in 0..customers {
        let customer_corr_id = format!("{}-customer-{}-{}", tenant, seed, customer_idx);
        entries.push(serde_json::json!({
            "type": "customer",
            "correlation_id": customer_corr_id,
        }));

        for invoice_idx in 0..invoices_per_customer {
            let invoice_corr_id = format!(
                "{}-invoice-{}-{}",
                tenant,
                seed,
                customer_idx * 100 + invoice_idx
            );
            let amount_cents: i32 = rng.gen_range(1000..=50000);
            let _due_days: u32 = rng.gen_range(14..=60);
            entries.push(serde_json::json!({
                "type": "invoice",
                "correlation_id": invoice_corr_id,
                "amount_cents": amount_cents,
            }));
        }
    }

    entries.sort_by(|a, b| {
        let ta = a["type"].as_str().unwrap_or("");
        let tb = b["type"].as_str().unwrap_or("");
        let ca = a["correlation_id"].as_str().unwrap_or("");
        let cb = b["correlation_id"].as_str().unwrap_or("");
        ta.cmp(tb).then(ca.cmp(cb))
    });

    let json = serde_json::to_string(&entries).expect("Failed to serialize");
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic() {
        let d1 = expected_digest("t1", 42, 2, 3);
        let d2 = expected_digest("t1", 42, 2, 3);
        assert_eq!(d1, d2, "Same parameters should produce same digest");
    }

    #[test]
    fn different_seeds_produce_different_digests() {
        let d1 = expected_digest("t1", 42, 2, 3);
        let d2 = expected_digest("t1", 99, 2, 3);
        assert_ne!(d1, d2, "Different seeds should produce different digests");
    }

    #[test]
    fn different_tenants_produce_different_digests() {
        let d1 = expected_digest("t1", 42, 2, 3);
        let d2 = expected_digest("t2", 42, 2, 3);
        assert_ne!(d1, d2, "Different tenants should produce different digests");
    }

    #[test]
    fn digest_tracker_empty_is_deterministic() {
        let t = DigestTracker::new();
        let h1 = t.finalize();
        let t2 = DigestTracker::new();
        let h2 = t2.finalize();
        assert_eq!(h1, h2, "Empty trackers should produce same digest");
    }

    #[test]
    fn digest_tracker_records_and_finalizes() {
        let mut t = DigestTracker::new();
        t.record_customer(1, "t1-customer-42-0");
        t.record_invoice(10, "t1-invoice-42-0", 5000);
        let digest = t.finalize();
        assert!(!digest.is_empty());
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_tracker_order_independent() {
        let mut t1 = DigestTracker::new();
        t1.record_customer(1, "corr-a");
        t1.record_invoice(10, "corr-b", 5000);

        let mut t2 = DigestTracker::new();
        // Insert in different order
        t2.record_invoice(10, "corr-b", 5000);
        t2.record_customer(1, "corr-a");

        // Should sort to same canonical representation
        assert_eq!(
            t1.finalize(),
            t2.finalize(),
            "Digest should be order-independent"
        );
    }

    #[test]
    fn digest_tracker_numbering_policy() {
        let mut t = DigestTracker::new();
        t.record_numbering_policy("purchase-order", "PO");
        t.record_numbering_policy("sales-order", "SO");
        let digest = t.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");

        // Same policies in different order should produce same digest
        let mut t2 = DigestTracker::new();
        t2.record_numbering_policy("sales-order", "SO");
        t2.record_numbering_policy("purchase-order", "PO");
        assert_eq!(digest, t2.finalize(), "Numbering digest should be order-independent");
    }

    #[test]
    fn digest_tracker_gl_account() {
        let mut t = DigestTracker::new();
        t.record_gl_account("1200", "Raw Materials Inventory");
        t.record_gl_account("5000", "COGS - Direct Materials");
        let digest = t.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");

        // Order-independent
        let mut t2 = DigestTracker::new();
        t2.record_gl_account("5000", "COGS - Direct Materials");
        t2.record_gl_account("1200", "Raw Materials Inventory");
        assert_eq!(digest, t2.finalize(), "GL account digest should be order-independent");
    }

    #[test]
    fn digest_tracker_fx_rate() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();

        let mut t = DigestTracker::new();
        t.record_fx_rate(id1, "USD/EUR");
        t.record_fx_rate(id2, "USD/GBP");
        let digest = t.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");

        // Order-independent
        let mut t2 = DigestTracker::new();
        t2.record_fx_rate(id2, "USD/GBP");
        t2.record_fx_rate(id1, "USD/EUR");
        assert_eq!(digest, t2.finalize(), "FX rate digest should be order-independent");
    }

    #[test]
    fn digest_tracker_party() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();

        let mut t = DigestTracker::new();
        t.record_party(id1, "Boeing Defense", "customer");
        t.record_party(id2, "Alcoa", "supplier");
        let digest = t.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");

        // Order-independent
        let mut t2 = DigestTracker::new();
        t2.record_party(id2, "Alcoa", "supplier");
        t2.record_party(id1, "Boeing Defense", "customer");
        assert_eq!(digest, t2.finalize(), "Party digest should be order-independent");
    }
}
