//! Shipping notification consumers.
//!
//! Handles `shipping_receiving.outbound_shipped` and
//! `shipping_receiving.outbound_delivered` events and creates template-based
//! notification send requests.
//!
//! Idempotency is enforced upstream via the `processed_events` gate in the
//! SDK consumer adapters registered in `main.rs`.
//!
//! Payload types mirror `shipping_receiving::events::contracts` to avoid a
//! cross-crate dependency on the shipping-receiving library.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::EnvelopeMetadata;
use crate::sends::repo as sends_repo;
use crate::templates::tracking_url as compute_tracking_url;

// ── Payload types ─────────────────────────────────────────────────────────────

/// Mirrored from `shipping_receiving::events::contracts::OutboundShippedLine`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundShippedLine {
    pub line_id: Uuid,
    pub sku: String,
    pub qty_shipped: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<Uuid>,
    /// Source document type, e.g. "sales_order"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref_type: Option<String>,
    /// Source document ID, e.g. the sales order UUID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref_id: Option<Uuid>,
}

/// Mirrored from `shipping_receiving::events::contracts::OutboundShippedPayload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundShippedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub lines: Vec<OutboundShippedLine>,
    pub shipped_at: DateTime<Utc>,
    /// Carrier tracking number — None if not yet assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_number: Option<String>,
    /// Party UUID of the carrier — None if not known at ship time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_party_id: Option<Uuid>,
}

/// Mirrored from `shipping_receiving::events::contracts::OutboundDeliveredPayload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundDeliveredPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub delivered_at: DateTime<Utc>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Handle `shipping_receiving.outbound_shipped`.
///
/// Creates a `notification_sends` row with `template_key = "order_shipped"`.
/// Idempotency is enforced upstream by the `processed_events` gate.
///
/// Defaults:
/// - `tracking_number = None` → template var `"pending"`
/// - `carrier_party_id = None` → template var `"unknown"`
///
/// Customer resolution: uses the first line's `source_ref_id` (the sales order
/// UUID) as the recipient ref. Falls back to `shipment:{id}` if no source_ref
/// is present, which is resolvable at delivery time via the party module.
pub async fn handle_outbound_shipped(
    pool: &PgPool,
    payload: OutboundShippedPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        shipment_id = %payload.shipment_id,
        tenant_id = %payload.tenant_id,
        tracking_number = ?payload.tracking_number,
        "Handling outbound_shipped shipping notification"
    );

    let tracking_number = payload
        .tracking_number
        .as_deref()
        .unwrap_or("pending")
        .to_string();

    let carrier = payload
        .carrier_party_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let shipped_at = payload.shipped_at.to_rfc3339();

    // Compute carrier tracking URL. Returns empty string when carrier code is
    // unrecognised (e.g. a party UUID) or tracking number is absent — the
    // template must always receive a string value, never null or a missing key.
    let url = compute_tracking_url(&carrier, &tracking_number).unwrap_or_default();

    // Resolve recipient from the first line with a source_ref_id (sales order ref).
    // Falls back to shipment-keyed ref for downstream resolution by party module.
    let recipient = payload
        .lines
        .iter()
        .filter_map(|line| line.source_ref_id.map(|id| format!("sales_order:{}", id)))
        .next()
        .unwrap_or_else(|| format!("shipment:{}", payload.shipment_id));

    let payload_json = serde_json::json!({
        "tracking_number": tracking_number,
        "carrier": carrier,
        "shipped_at": shipped_at,
        "recipient_name": "Customer",
        "shipment_id": payload.shipment_id.to_string(),
        "tracking_url": url,
    });

    let send = sends_repo::insert_send(
        pool,
        &payload.tenant_id,
        Some("order_shipped"),
        None, // latest version
        "email",
        &[recipient.clone()],
        &payload_json,
        metadata.correlation_id.as_deref(),
        Some(&metadata.event_id.to_string()),
        None,
    )
    .await?;

    tracing::info!(
        send_id = %send.id,
        shipment_id = %payload.shipment_id,
        recipient = %recipient,
        tracking_number = %tracking_number,
        tracking_url = %url,
        "order_shipped send request created"
    );

    Ok(())
}

/// Handle `shipping_receiving.outbound_delivered`.
///
/// Creates a `notification_sends` row with `template_key = "delivery_confirmed"`.
/// Idempotency is enforced upstream by the `processed_events` gate.
pub async fn handle_outbound_delivered(
    pool: &PgPool,
    payload: OutboundDeliveredPayload,
    metadata: EnvelopeMetadata,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        shipment_id = %payload.shipment_id,
        tenant_id = %payload.tenant_id,
        "Handling outbound_delivered shipping notification"
    );

    let delivered_at = payload.delivered_at.to_rfc3339();
    let recipient = format!("shipment:{}", payload.shipment_id);

    let payload_json = serde_json::json!({
        "delivered_at": delivered_at,
        "recipient_name": "Customer",
        "shipment_id": payload.shipment_id.to_string(),
    });

    let send = sends_repo::insert_send(
        pool,
        &payload.tenant_id,
        Some("delivery_confirmed"),
        None, // latest version
        "email",
        &[recipient.clone()],
        &payload_json,
        metadata.correlation_id.as_deref(),
        Some(&metadata.event_id.to_string()),
        None,
    )
    .await?;

    tracing::info!(
        send_id = %send.id,
        shipment_id = %payload.shipment_id,
        "delivery_confirmed send request created"
    );

    Ok(())
}
