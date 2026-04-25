//! Canonical carrier tracking events — persistence and status recomputation.
//!
//! Invariant: tracking events inform visibility; they NEVER advance the shipment
//! state machine (draft → confirmed → in_transit, etc.). State advances require
//! dock-scan or manual-receipt API calls only (per 2026-04-24 decision).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub mod odfl_poller;

// ── Canonical status vocabulary ───────────────────────────────

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_PICKED_UP: &str = "picked_up";
pub const STATUS_IN_TRANSIT: &str = "in_transit";
pub const STATUS_OUT_FOR_DELIVERY: &str = "out_for_delivery";
pub const STATUS_DELIVERED: &str = "delivered";
pub const STATUS_EXCEPTION: &str = "exception";
pub const STATUS_RETURNED: &str = "returned";
pub const STATUS_LOST: &str = "lost";

/// Advancement rank for the multi-package "least advanced" rule.
///
/// exception / returned / lost rank 0 — they dominate immediately (a single
/// damaged package makes the master reflect that). All other statuses rank
/// by forward progress through the delivery lifecycle.
pub fn status_rank(s: &str) -> u8 {
    match s {
        STATUS_EXCEPTION | STATUS_RETURNED | STATUS_LOST => 0,
        STATUS_PENDING => 1,
        STATUS_PICKED_UP => 2,
        STATUS_IN_TRANSIT => 3,
        STATUS_OUT_FOR_DELIVERY => 4,
        STATUS_DELIVERED => 5,
        _ => 0,
    }
}

// ── Persistence ───────────────────────────────────────────────

/// Persist a canonical tracking event atomically.
///
/// Returns the new row ID on first write, or `None` if an identical event
/// (same tracking_number + carrier_code + raw_payload_hash) was already
/// recorded — idempotent webhook replay is a silent no-op.
pub async fn record_tracking_event(
    pool: &PgPool,
    tenant_id: &str,
    shipment_id: Option<Uuid>,
    tracking_number: &str,
    carrier_code: &str,
    status: &str,
    status_dttm: DateTime<Utc>,
    location: Option<&str>,
    raw_payload_hash: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"
        INSERT INTO tracking_events
            (tenant_id, shipment_id, tracking_number, carrier_code,
             status, status_dttm, location, raw_payload_hash)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (tracking_number, carrier_code, raw_payload_hash) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(tracking_number)
    .bind(carrier_code)
    .bind(status)
    .bind(status_dttm)
    .bind(location)
    .bind(raw_payload_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(id,)| id))
}

/// Update a shipment's `carrier_status` to the latest carrier-reported value.
pub async fn update_shipment_carrier_status(
    pool: &PgPool,
    shipment_id: Uuid,
    new_status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE shipments
           SET carrier_status            = $1,
               carrier_status_updated_at = now()
         WHERE id = $2
        "#,
    )
    .bind(new_status)
    .bind(shipment_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Recompute the master shipment's `carrier_status` as the "least advanced"
/// status across all child shipments.
///
/// Rule: master = child with the lowest `status_rank`. This means one
/// in-transit child keeps the master in_transit even if others are delivered,
/// and one excepted child immediately surfaces the exception at master level.
pub async fn recompute_master_status(
    pool: &PgPool,
    master_shipment_id: Uuid,
) -> Result<(), sqlx::Error> {
    let children: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT carrier_status FROM shipments WHERE parent_shipment_id = $1",
    )
    .bind(master_shipment_id)
    .fetch_all(pool)
    .await?;

    if children.is_empty() {
        return Ok(());
    }

    let mut min_rank: u8 = u8::MAX;
    let mut min_status: &str = STATUS_PENDING;

    for (status_opt,) in &children {
        let s = status_opt.as_deref().unwrap_or(STATUS_PENDING);
        let rank = status_rank(s);
        if rank < min_rank {
            min_rank = rank;
            min_status = s;
        }
    }

    update_shipment_carrier_status(pool, master_shipment_id, min_status).await
}

/// Look up a shipment by tracking number.
///
/// Returns `(shipment_id, tenant_id, parent_shipment_id)` or `None`.
pub async fn find_shipment_by_tracking(
    pool: &PgPool,
    tracking_number: &str,
) -> Result<Option<(Uuid, String, Option<Uuid>)>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT id, tenant_id::text, parent_shipment_id
          FROM shipments
         WHERE tracking_number = $1
         LIMIT 1
        "#,
    )
    .bind(tracking_number)
    .fetch_optional(pool)
    .await
}

// ── Hash utility ──────────────────────────────────────────────

/// Compute SHA-256 hex digest of `data`. Used to populate `raw_payload_hash`.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

// ── Unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_rank_delivered_beats_in_transit() {
        assert!(status_rank(STATUS_DELIVERED) > status_rank(STATUS_IN_TRANSIT));
    }

    #[test]
    fn exception_has_lowest_rank() {
        assert_eq!(status_rank(STATUS_EXCEPTION), 0);
        assert_eq!(status_rank(STATUS_RETURNED), 0);
        assert_eq!(status_rank(STATUS_LOST), 0);
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let h1 = sha256_hex(b"hello");
        let h2 = sha256_hex(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }
}
