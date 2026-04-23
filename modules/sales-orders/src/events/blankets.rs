//! Blanket order and release event contracts.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE, MUTATION_CLASS_REVERSAL,
    SO_EVENT_SCHEMA_VERSION,
};
use crate::events::envelope::{create_so_envelope, EventEnvelope};

// ── Event type constants ──────────────────────────────────────────────────────

pub const EVENT_TYPE_BLANKET_ACTIVATED: &str = "sales_orders.blanket_activated";
pub const EVENT_TYPE_BLANKET_EXPIRED: &str = "sales_orders.blanket_expired";
pub const EVENT_TYPE_RELEASE_CREATED: &str = "sales_orders.release_created";

// ── Payloads ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlanketActivatedPayload {
    pub blanket_order_id: Uuid,
    pub blanket_order_number: String,
    pub customer_id: Option<Uuid>,
    pub total_committed_value_cents: i64,
    pub valid_until: Option<NaiveDate>,
    pub tenant_id: String,
    pub activated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlanketExpiredPayload {
    pub blanket_order_id: Uuid,
    pub blanket_order_number: String,
    pub tenant_id: String,
    pub expired_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseCreatedPayload {
    pub release_id: Uuid,
    pub blanket_order_id: Uuid,
    pub blanket_order_line_id: Uuid,
    pub release_qty: f64,
    pub sales_order_id: Uuid,
    pub tenant_id: String,
    pub created_at: DateTime<Utc>,
}

// ── Envelope builders ─────────────────────────────────────────────────────────

pub fn build_blanket_activated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: BlanketActivatedPayload,
) -> EventEnvelope<BlanketActivatedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_BLANKET_ACTIVATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_blanket_expired_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: BlanketExpiredPayload,
) -> EventEnvelope<BlanketExpiredPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_BLANKET_EXPIRED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_release_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReleaseCreatedPayload,
) -> EventEnvelope<ReleaseCreatedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RELEASE_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

/// Blanket cancelled payload — for when a blanket order is explicitly cancelled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlanketCancelledPayload {
    pub blanket_order_id: Uuid,
    pub blanket_order_number: String,
    pub tenant_id: String,
    pub cancelled_at: DateTime<Utc>,
}

pub const EVENT_TYPE_BLANKET_CANCELLED: &str = "sales_orders.blanket_cancelled";

pub fn build_blanket_cancelled_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: BlanketCancelledPayload,
) -> EventEnvelope<BlanketCancelledPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_BLANKET_CANCELLED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}
