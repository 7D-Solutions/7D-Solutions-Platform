//! Customer lifecycle event contracts.
//!
//! ar.customer_created — emitted when an AR customer record is inserted.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

/// Emitted when an AR customer record is created.
pub const EVENT_TYPE_AR_CUSTOMER_CREATED: &str = "ar.customer_created";

/// Payload for ar.customer_created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArCustomerCreatedPayload {
    pub customer_id: i32,
    pub tenant_id: String,
    pub email: String,
    pub name: Option<String>,
    pub party_id: Option<Uuid>,
}

/// Build envelope for ar.customer_created (mutation_class: LIFECYCLE).
pub fn build_ar_customer_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ArCustomerCreatedPayload,
) -> EventEnvelope<ArCustomerCreatedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_AR_CUSTOMER_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}
