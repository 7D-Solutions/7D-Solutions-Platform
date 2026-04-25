//! ODFL tracking poll task — runs every 15 minutes for in-transit ODFL shipments.
//!
//! ODFL does not offer webhook push. This poller calls the existing
//! OdflCarrierProvider::track() and records results as tracking_events using
//! the same canonical pipeline as webhook handlers.
//!
//! Pattern: new carriers that lack webhook support can use this same module —
//! implement CarrierProvider::track() and add an entry here.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::carrier_providers::{credentials::get_carrier_credentials, get_provider};
use crate::domain::tracking::{self, sha256_hex};

const POLL_INTERVAL_SECS: u64 = 15 * 60;
const POLL_BATCH_SIZE: i64 = 200;

/// Spawn a background Tokio task that polls ODFL tracking every 15 minutes.
pub fn start_odfl_poll_task(pool: Arc<PgPool>) {
    tokio::spawn(async move {
        tracing::info!("SR: ODFL tracking poller started (interval={}s)", POLL_INTERVAL_SECS);
        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        let client = Client::new();
        loop {
            ticker.tick().await;
            if let Err(e) = run_poll_cycle(&pool, &client).await {
                tracing::error!(error = %e, "SR: ODFL poll cycle failed");
            }
        }
    });
}

async fn run_poll_cycle(
    pool: &PgPool,
    client: &Client,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Find ODFL shipments that are not yet terminal (delivered/exception/returned/lost).
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        r#"
        SELECT s.id, s.tenant_id::text, s.tracking_number
          FROM shipments s
         WHERE s.tracking_number IS NOT NULL
           AND (s.carrier_status IS NULL
                OR s.carrier_status IN ('pending','picked_up','in_transit','out_for_delivery'))
           AND EXISTS (
               SELECT 1 FROM sr_carrier_requests r
                WHERE r.shipment_id = s.id AND r.carrier_code = 'odfl'
           )
         ORDER BY s.id
         LIMIT $1
        "#,
    )
    .bind(POLL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let provider = match get_provider("odfl") {
        Some(p) => p,
        None => return Ok(()),
    };

    for (shipment_id, tenant_id, tracking_number) in rows {
        let config = match get_carrier_credentials(client, &tenant_id, "odfl").await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    error = %e,
                    "SR: ODFL poll: no credentials for tenant"
                );
                continue;
            }
        };

        let result = match provider.track(&tracking_number, &config).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    tracking_number = %tracking_number,
                    error = %e,
                    "SR: ODFL poll: track call failed"
                );
                continue;
            }
        };

        let canonical_status = odfl_status_to_canonical(&result.status);
        // Hash the tracking number + raw status string for idempotency
        let hash_input = format!("{}:{}", tracking_number, result.status);
        let hash = sha256_hex(hash_input.as_bytes());
        let now = chrono::Utc::now();

        let inserted = tracking::record_tracking_event(
            pool,
            &tenant_id,
            Some(shipment_id),
            &tracking_number,
            "odfl",
            canonical_status,
            now,
            result.location.as_deref(),
            &hash,
        )
        .await
        .unwrap_or(None);

        if inserted.is_none() {
            // Already recorded for this status — skip status update
            continue;
        }

        let _ = tracking::update_shipment_carrier_status(pool, shipment_id, canonical_status).await;

        tracing::debug!(
            tracking_number = %tracking_number,
            status = %canonical_status,
            "SR: ODFL poll: tracking event recorded"
        );
    }

    Ok(())
}

fn odfl_status_to_canonical(s: &str) -> &'static str {
    let upper = s.to_ascii_uppercase();
    if upper.contains("DELIVER") {
        tracking::STATUS_DELIVERED
    } else if upper.contains("OUT FOR") || upper.contains("OFD") {
        tracking::STATUS_OUT_FOR_DELIVERY
    } else if upper.contains("PICKUP") || upper.contains("PICKED") || upper.contains("PUP") {
        tracking::STATUS_PICKED_UP
    } else if upper.contains("EXCEPTION") || upper.contains("DAMAGE") {
        tracking::STATUS_EXCEPTION
    } else if upper.contains("RETURN") {
        tracking::STATUS_RETURNED
    } else {
        // In-transit is the safe fallback for anything showing movement
        tracking::STATUS_IN_TRANSIT
    }
}
