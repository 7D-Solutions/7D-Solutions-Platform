//! Warehouse location seeding for demo-seed

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateLocationRequest {
    tenant_id: String,
    warehouse_id: Uuid,
    code: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocationResponse {
    id: Uuid,
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

pub(super) struct LocationDef {
    pub code: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub(super) const LOCATIONS: &[LocationDef] = &[
    LocationDef { code: "RECV-DOCK", name: "Receiving Dock", description: "Inbound material receiving area" },
    LocationDef { code: "RAW-WH", name: "Raw Material Warehouse", description: "Bulk raw material storage" },
    LocationDef { code: "WIP-FLOOR", name: "WIP Production Floor", description: "Active production work-in-progress area" },
    LocationDef { code: "FG-WH", name: "Finished Goods Warehouse", description: "Completed product storage" },
    LocationDef { code: "SHIP-DOCK", name: "Shipping Dock", description: "Outbound shipping and dispatch area" },
    LocationDef { code: "QA-HOLD", name: "Quality Hold Area", description: "Quarantine area for quality inspection" },
    LocationDef { code: "MRB", name: "Material Review Board", description: "Non-conforming material review and disposition" },
];

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

async fn create_location(
    client: &reqwest::Client,
    inventory_url: &str,
    tenant: &str,
    wh_id: Uuid,
    loc: &LocationDef,
) -> Result<Option<Uuid>> {
    let url = format!("{}/api/inventory/locations", inventory_url);

    let body = CreateLocationRequest {
        tenant_id: tenant.to_string(),
        warehouse_id: wh_id,
        code: loc.code.to_string(),
        name: loc.name.to_string(),
        description: Some(loc.description.to_string()),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/inventory/locations ({}) network error", loc.code))?;

    let status = resp.status();

    if status == reqwest::StatusCode::CONFLICT {
        info!(code = loc.code, "Location already exists");
        return Ok(None);
    }

    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let loc_resp: LocationResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse location response for {}", loc.code))?;
        return Ok(Some(loc_resp.id));
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/inventory/locations ({}) failed {status}: {text}",
        loc.code
    );
}

// ---------------------------------------------------------------------------
// Seeding logic
// ---------------------------------------------------------------------------

pub(super) async fn seed_locations(
    client: &reqwest::Client,
    inventory_url: &str,
    tenant: &str,
    wh_id: Uuid,
    tracker: &mut DigestTracker,
) -> Result<Vec<(Uuid, String)>> {
    let mut locations = Vec::with_capacity(LOCATIONS.len());
    for loc in LOCATIONS {
        let maybe_id = create_location(client, inventory_url, tenant, wh_id, loc).await?;
        let loc_id = match maybe_id {
            Some(id) => id,
            None => {
                // 409 — use deterministic placeholder
                Uuid::new_v5(
                    &Uuid::NAMESPACE_DNS,
                    format!("{}-location-{}", tenant, loc.code).as_bytes(),
                )
            }
        };
        tracker.record_location(loc_id, loc.code);
        locations.push((loc_id, loc.code.to_string()));
        info!(code = loc.code, name = loc.name, location_id = %loc_id, "Location seeded");
    }
    Ok(locations)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_locations_defined() {
        assert_eq!(LOCATIONS.len(), 7, "Expected 7 locations");
    }

    #[test]
    fn location_codes_are_unique() {
        let mut codes: Vec<&str> = LOCATIONS.iter().map(|l| l.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), LOCATIONS.len(), "Duplicate location codes found");
    }

    #[test]
    fn digest_records_locations() {
        let mut tracker = DigestTracker::new();
        let id = Uuid::new_v4();
        tracker.record_location(id, "RECV-DOCK");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }
}
