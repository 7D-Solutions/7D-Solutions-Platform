//! UoM (Unit of Measure) seeding for demo-seed

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateUomRequest {
    tenant_id: String,
    code: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct UomResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct UomListEntry {
    id: Uuid,
    code: String,
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

pub(super) struct UomDef {
    pub code: &'static str,
    pub name: &'static str,
}

pub(super) const UOMS: &[UomDef] = &[
    UomDef { code: "EA", name: "Each" },
    UomDef { code: "KG", name: "Kilogram" },
    UomDef { code: "LB", name: "Pound" },
    UomDef { code: "M", name: "Meter" },
    UomDef { code: "IN", name: "Inch" },
];

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

async fn create_uom(
    client: &reqwest::Client,
    inventory_url: &str,
    tenant: &str,
    uom: &UomDef,
) -> Result<Option<Uuid>> {
    let url = format!("{}/api/inventory/uoms", inventory_url);

    let body = CreateUomRequest {
        tenant_id: tenant.to_string(),
        code: uom.code.to_string(),
        name: uom.name.to_string(),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/inventory/uoms ({}) network error", uom.code))?;

    let status = resp.status();

    if status == reqwest::StatusCode::CONFLICT {
        info!(code = uom.code, "UoM already exists — retrieving real UUID");
        let real_id = find_uom_by_code(client, inventory_url, uom.code).await?;
        return Ok(Some(real_id));
    }

    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let uom_resp: UomResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse UoM response for {}", uom.code))?;
        return Ok(Some(uom_resp.id));
    }

    let text = resp.text().await.unwrap_or_default();
    bail!("POST /api/inventory/uoms ({}) failed {status}: {text}", uom.code);
}

/// Fetch all UoMs and find one by exact code match.
async fn find_uom_by_code(
    client: &reqwest::Client,
    inventory_url: &str,
    code: &str,
) -> Result<Uuid> {
    let url = format!("{}/api/inventory/uoms", inventory_url);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| "GET /api/inventory/uoms network error".to_string())?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("GET /api/inventory/uoms failed {status}: {text}");
    }

    let uoms: Vec<UomListEntry> = resp
        .json()
        .await
        .context("Failed to parse UoM list response")?;

    for entry in &uoms {
        if entry.code == code {
            return Ok(entry.id);
        }
    }

    bail!("UoM with code '{}' returned 409 but could not be found via list", code);
}

// ---------------------------------------------------------------------------
// Seeding logic
// ---------------------------------------------------------------------------

pub(super) async fn seed_uoms(
    client: &reqwest::Client,
    inventory_url: &str,
    tenant: &str,
    tracker: &mut DigestTracker,
) -> Result<(Vec<(Uuid, String)>, usize)> {
    let mut uom_count = 0;
    let mut uoms = Vec::with_capacity(UOMS.len());
    for uom in UOMS {
        let uom_id = create_uom(client, inventory_url, tenant, uom)
            .await?
            .expect("create_uom always returns Some after 409 recovery");
        tracker.record_uom(uom_id, uom.code);
        uoms.push((uom_id, uom.code.to_string()));
        uom_count += 1;
        info!(code = uom.code, name = uom.name, "UoM seeded");
    }
    Ok((uoms, uom_count))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_uoms_defined() {
        assert_eq!(UOMS.len(), 5, "Expected 5 UoMs");
    }

    #[test]
    fn uom_codes_are_unique() {
        let mut codes: Vec<&str> = UOMS.iter().map(|u| u.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), UOMS.len(), "Duplicate UoM codes found");
    }

    #[test]
    fn digest_records_uoms() {
        let mut tracker = DigestTracker::new();
        let id = Uuid::new_v4();
        tracker.record_uom(id, "EA");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }
}
