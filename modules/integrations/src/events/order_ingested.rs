//! Event contract: integrations.order.ingested
//!
//! Emitted by `ShopifyNormalizer` (and future marketplace normalizers) after a
//! platform-standard order has been extracted from a marketplace webhook and a
//! `file_job` record has been durably created.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_ORDER_INGESTED: &str = "integrations.order.ingested";

/// A single line item on the order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderLineItemPayload {
    pub product_id: String,
    pub variant_id: String,
    pub title: String,
    pub quantity: u32,
    pub price: String,
    pub sku: Option<String>,
}

/// Payload for the `integrations.order.ingested` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderIngestedPayload {
    /// Tenant that owns the connector.
    pub tenant_id: String,
    /// Source system identifier (e.g. `"shopify"`).
    pub source: String,
    /// Source-system order ID.
    pub order_id: String,
    /// Human-readable order number (source-specific).
    pub order_number: Option<u64>,
    /// Financial status at the time of ingestion.
    pub financial_status: Option<String>,
    /// Normalised line items.
    pub line_items: Vec<OrderLineItemPayload>,
    /// Customer email or external reference, if available.
    pub customer_ref: Option<String>,
    /// File job created to durably track processing.
    pub file_job_id: Uuid,
    pub ingested_at: DateTime<Utc>,
}

pub fn build_order_ingested_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderIngestedPayload,
) -> EventEnvelope<OrderIngestedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ORDER_INGESTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_ingested_envelope_metadata() {
        let payload = OrderIngestedPayload {
            tenant_id: "t-1".to_string(),
            source: "shopify".to_string(),
            order_id: "5678901234".to_string(),
            order_number: Some(1001),
            financial_status: Some("paid".to_string()),
            line_items: vec![],
            customer_ref: Some("alice@example.com".to_string()),
            file_job_id: Uuid::new_v4(),
            ingested_at: Utc::now(),
        };
        let env = build_order_ingested_envelope(
            Uuid::new_v4(),
            "t-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_ORDER_INGESTED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
    }
}
