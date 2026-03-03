//! Event contract for inventory.expiry_set.v1.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::{create_inventory_envelope, MUTATION_CLASS_DATA_MUTATION};
use event_bus::EventEnvelope;

pub const EVENT_TYPE_EXPIRY_SET: &str = "inventory.expiry_set.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpirySetPayload {
    pub tenant_id: String,
    pub lot_id: Uuid,
    pub item_id: Uuid,
    pub lot_code: String,
    pub expiry_date: NaiveDate,
    pub source: String,
    pub set_at: DateTime<Utc>,
}

pub fn build_expiry_set_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ExpirySetPayload,
) -> EventEnvelope<ExpirySetPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_EXPIRY_SET.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
}
