//! BOM (Bill of Materials) module seeder for demo-seed
//!
//! Creates bills of materials with revisions and component lines for
//! aerospace manufacturing items. Depends on inventory items existing first.
//!
//! Uses GET-before-create pattern for idempotent re-runs:
//! 1. GET /api/bom/by-part/{part_id} to check if BOM exists
//! 2. GET /api/bom/{bom_id}/revisions to check if revision A exists
//! 3. GET /api/bom/revisions/{revision_id}/lines to check existing lines

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// Fixed effectivity date (deterministic across runs)
// ---------------------------------------------------------------------------

fn effectivity_from() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("Fixed effectivity date must parse")
}

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateBomRequest {
    part_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BomHeaderResponse {
    id: Uuid,
}

#[derive(Serialize)]
struct CreateRevisionRequest {
    revision_label: String,
}

#[derive(Debug, Deserialize)]
struct RevisionResponse {
    id: Uuid,
    revision_label: String,
}

#[derive(Serialize)]
struct AddLineRequest {
    component_item_id: Uuid,
    quantity: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    uom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scrap_factor: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    find_number: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct LineResponse {
    id: Uuid,
    component_item_id: Uuid,
}

#[derive(Serialize)]
struct SetEffectivityRequest {
    effective_from: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_to: Option<DateTime<Utc>>,
}

// Inventory search types (for standalone --modules bom)

#[derive(Debug, Deserialize)]
struct InventorySearchItem {
    id: Uuid,
    sku: String,
}

#[derive(Debug, Deserialize)]
struct InventorySearchResponse {
    data: Vec<InventorySearchItem>,
}

// ---------------------------------------------------------------------------
// Static BOM structure definitions
// ---------------------------------------------------------------------------

struct BomComponentDef {
    component_sku: &'static str,
    quantity: f64,
    uom: Option<&'static str>,
    scrap_factor: Option<f64>,
    find_number: i32,
}

struct BomDef {
    make_item_sku: &'static str,
    description: &'static str,
    components: &'static [BomComponentDef],
}

const BOMS: &[BomDef] = &[
    BomDef {
        make_item_sku: "TBB-ASSY-001",
        description: "Turbine Blade Blank BOM",
        components: &[BomComponentDef {
            component_sku: "TI64-BAR-001",
            quantity: 1.0,
            uom: Some("KG"),
            scrap_factor: Some(0.05),
            find_number: 10,
        }],
    },
    BomDef {
        make_item_sku: "EMB-ASSY-001",
        description: "Engine Mount Bracket BOM",
        components: &[
            BomComponentDef {
                component_sku: "AL7075-SHT-001",
                quantity: 2.0,
                uom: Some("KG"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "AN3-BOLT",
                quantity: 4.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 20,
            },
            BomComponentDef {
                component_sku: "MS21042-NUT",
                quantity: 4.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 30,
            },
            BomComponentDef {
                component_sku: "NAS1149-WASH",
                quantity: 8.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 40,
            },
        ],
    },
    BomDef {
        make_item_sku: "SRA-ASSY-001",
        description: "Structural Rib Assembly BOM",
        components: &[
            BomComponentDef {
                component_sku: "4130-TUB-001",
                quantity: 3.0,
                uom: Some("M"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "INC718-FRG-001",
                quantity: 1.0,
                uom: Some("KG"),
                scrap_factor: Some(0.03),
                find_number: 20,
            },
        ],
    },
    BomDef {
        make_item_sku: "FLC-ASSY-001",
        description: "Fuel Line Connector BOM",
        components: &[
            BomComponentDef {
                component_sku: "4130-TUB-001",
                quantity: 1.0,
                uom: Some("M"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "AN3-BOLT",
                quantity: 2.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 20,
            },
        ],
    },
    BomDef {
        make_item_sku: "LGA-ASSY-001",
        description: "Landing Gear Actuator Housing BOM",
        components: &[
            BomComponentDef {
                component_sku: "INC718-FRG-001",
                quantity: 2.0,
                uom: Some("KG"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "TI64-BAR-001",
                quantity: 1.0,
                uom: Some("KG"),
                scrap_factor: None,
                find_number: 20,
            },
        ],
    },
];

/// All SKUs referenced in BOM definitions (make items + components)
const ALL_REFERENCED_SKUS: &[&str] = &[
    "TBB-ASSY-001",
    "EMB-ASSY-001",
    "SRA-ASSY-001",
    "FLC-ASSY-001",
    "LGA-ASSY-001",
    "TI64-BAR-001",
    "INC718-FRG-001",
    "AL7075-SHT-001",
    "4130-TUB-001",
    "AN3-BOLT",
    "MS21042-NUT",
    "NAS1149-WASH",
];

// ---------------------------------------------------------------------------
// Public return type
// ---------------------------------------------------------------------------

/// Created BOM resource IDs for downstream modules (production)
pub struct BomIds {
    /// (bom_id, part_id, make_item_sku)
    pub boms: Vec<(Uuid, Uuid, String)>,
    /// (revision_id, bom_id)
    pub revisions: Vec<(Uuid, Uuid)>,
}

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

/// Check if a BOM already exists for this part, or create one.
async fn get_or_create_bom(
    client: &reqwest::Client,
    bom_url: &str,
    part_id: Uuid,
    description: &str,
) -> Result<Uuid> {
    // GET /api/bom/by-part/{part_id}
    let get_url = format!("{}/api/bom/by-part/{}", bom_url, part_id);
    let resp = client
        .get(&get_url)
        .send()
        .await
        .with_context(|| format!("GET /api/bom/by-part/{} network error", part_id))?;

    if resp.status().is_success() {
        let bom: BomHeaderResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse BOM by-part response for {}", part_id))?;
        info!(bom_id = %bom.id, part_id = %part_id, "BOM already exists");
        return Ok(bom.id);
    }

    if resp.status() != reqwest::StatusCode::NOT_FOUND {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/bom/by-part/{} unexpected {}: {}",
            part_id,
            status,
            text
        );
    }

    // 404 — create new BOM
    let post_url = format!("{}/api/bom", bom_url);
    let body = CreateBomRequest {
        part_id,
        description: Some(description.to_string()),
    };

    let resp = client
        .post(&post_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/bom for part {} network error", part_id))?;

    let status = resp.status();
    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let bom: BomHeaderResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse BOM creation response")?;
        return Ok(bom.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom for part {} failed {}: {}",
        part_id,
        status,
        text
    );
}

/// Check if revision A exists, or create it.
async fn get_or_create_revision(
    client: &reqwest::Client,
    bom_url: &str,
    bom_id: Uuid,
    label: &str,
) -> Result<Uuid> {
    // GET /api/bom/{bom_id}/revisions
    let list_url = format!("{}/api/bom/{}/revisions", bom_url, bom_id);
    let resp = client
        .get(&list_url)
        .send()
        .await
        .with_context(|| format!("GET /api/bom/{}/revisions network error", bom_id))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/bom/{}/revisions failed {}: {}",
            bom_id,
            status,
            text
        );
    }

    let revisions: Vec<RevisionResponse> = resp
        .json()
        .await
        .with_context(|| format!("Failed to parse revisions for BOM {}", bom_id))?;

    if let Some(rev) = revisions.iter().find(|r| r.revision_label == label) {
        info!(revision_id = %rev.id, label, "Revision already exists");
        return Ok(rev.id);
    }

    // Create revision
    let post_url = format!("{}/api/bom/{}/revisions", bom_url, bom_id);
    let body = CreateRevisionRequest {
        revision_label: label.to_string(),
    };

    let resp = client
        .post(&post_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/bom/{}/revisions network error", bom_id))?;

    let status = resp.status();
    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let rev: RevisionResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse revision creation response")?;
        return Ok(rev.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom/{}/revisions failed {}: {}",
        bom_id,
        status,
        text
    );
}

/// Get existing lines for a revision.
async fn get_revision_lines(
    client: &reqwest::Client,
    bom_url: &str,
    revision_id: Uuid,
) -> Result<Vec<LineResponse>> {
    let url = format!("{}/api/bom/revisions/{}/lines", bom_url, revision_id);
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET /api/bom/revisions/{}/lines network error", revision_id))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/bom/revisions/{}/lines failed {}: {}",
            revision_id,
            status,
            text
        );
    }

    let lines: Vec<LineResponse> = resp
        .json()
        .await
        .with_context(|| format!("Failed to parse lines for revision {}", revision_id))?;
    Ok(lines)
}

/// Add a component line to a revision.
async fn create_line(
    client: &reqwest::Client,
    bom_url: &str,
    revision_id: Uuid,
    component_item_id: Uuid,
    quantity: f64,
    uom: Option<&str>,
    scrap_factor: Option<f64>,
    find_number: i32,
) -> Result<Uuid> {
    let url = format!("{}/api/bom/revisions/{}/lines", bom_url, revision_id);
    let body = AddLineRequest {
        component_item_id,
        quantity,
        uom: uom.map(|s| s.to_string()),
        scrap_factor,
        find_number: Some(find_number),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| {
            format!(
                "POST /api/bom/revisions/{}/lines network error",
                revision_id
            )
        })?;

    let status = resp.status();
    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let line: LineResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse line creation response")?;
        return Ok(line.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom/revisions/{}/lines failed {}: {}",
        revision_id,
        status,
        text
    );
}

/// Set effectivity dates on a revision. Treats 409 as already-set success.
async fn set_effectivity(
    client: &reqwest::Client,
    bom_url: &str,
    revision_id: Uuid,
    effective_from: DateTime<Utc>,
) -> Result<()> {
    let url = format!("{}/api/bom/revisions/{}/effectivity", bom_url, revision_id);
    let body = SetEffectivityRequest {
        effective_from,
        effective_to: None,
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| {
            format!(
                "POST /api/bom/revisions/{}/effectivity network error",
                revision_id
            )
        })?;

    let status = resp.status();
    if status.is_success() || status == reqwest::StatusCode::CONFLICT {
        if status == reqwest::StatusCode::CONFLICT {
            info!(revision_id = %revision_id, "Effectivity already set");
        }
        return Ok(());
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom/revisions/{}/effectivity failed {}: {}",
        revision_id,
        status,
        text
    );
}

// ---------------------------------------------------------------------------
// Inventory item resolution (for standalone --modules bom)
// ---------------------------------------------------------------------------

/// Fetch all BOM-referenced items from the inventory service by SKU search.
/// Used when inventory module didn't run in this session.
pub async fn fetch_items_from_inventory(
    client: &reqwest::Client,
    inventory_url: &str,
) -> Result<Vec<(Uuid, String, String)>> {
    let mut items = Vec::with_capacity(ALL_REFERENCED_SKUS.len());

    for &sku in ALL_REFERENCED_SKUS {
        let url = format!("{}/api/inventory/items", inventory_url);
        let resp = client
            .get(&url)
            .query(&[("search", sku)])
            .send()
            .await
            .with_context(|| format!("GET /api/inventory/items?search={} network error", sku))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "GET /api/inventory/items?search={} failed {}: {}",
                sku,
                status,
                text
            );
        }

        let search: InventorySearchResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse item search for {}", sku))?;

        let item = search
            .data
            .iter()
            .find(|i| i.sku == sku)
            .with_context(|| {
                format!(
                    "Item '{}' not found in inventory — seed inventory first",
                    sku
                )
            })?;

        items.push((item.id, sku.to_string(), String::new()));
    }

    Ok(items)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Seed BOM data for all make items. Returns IDs for downstream modules.
pub async fn seed_boms(
    client: &reqwest::Client,
    bom_url: &str,
    items: &[(Uuid, String, String)],
    tracker: &mut DigestTracker,
) -> Result<BomIds> {
    // Build SKU -> UUID map
    let sku_map: HashMap<&str, Uuid> = items.iter().map(|(id, sku, _)| (sku.as_str(), *id)).collect();

    let mut result = BomIds {
        boms: Vec::with_capacity(BOMS.len()),
        revisions: Vec::with_capacity(BOMS.len()),
    };

    let eff_from = effectivity_from();

    for bom_def in BOMS {
        let part_id = *sku_map.get(bom_def.make_item_sku).with_context(|| {
            format!(
                "Make item '{}' not found in inventory items",
                bom_def.make_item_sku
            )
        })?;

        // Step 1: GET-before-create for BOM header
        let bom_id = get_or_create_bom(client, bom_url, part_id, bom_def.description).await?;
        tracker.record_bom(bom_id, part_id);
        result
            .boms
            .push((bom_id, part_id, bom_def.make_item_sku.to_string()));
        info!(bom_id = %bom_id, make_item = bom_def.make_item_sku, "BOM header ready");

        // Step 2: GET-before-create for revision A
        let revision_id = get_or_create_revision(client, bom_url, bom_id, "A").await?;
        result.revisions.push((revision_id, bom_id));
        info!(revision_id = %revision_id, bom_id = %bom_id, "Revision A ready");

        // Step 3: GET existing lines, create only missing ones
        let existing_lines = get_revision_lines(client, bom_url, revision_id).await?;
        let existing_component_ids: HashSet<Uuid> =
            existing_lines.iter().map(|l| l.component_item_id).collect();

        for comp in bom_def.components {
            let component_item_id = *sku_map.get(comp.component_sku).with_context(|| {
                format!(
                    "Component '{}' not found in inventory items",
                    comp.component_sku
                )
            })?;

            if existing_component_ids.contains(&component_item_id) {
                info!(component = comp.component_sku, "BOM line already exists — skipping");
                let existing = existing_lines
                    .iter()
                    .find(|l| l.component_item_id == component_item_id)
                    .expect("component must exist in existing lines");
                tracker.record_bom_line(existing.id, component_item_id, comp.quantity);
                continue;
            }

            let line_id = create_line(
                client,
                bom_url,
                revision_id,
                component_item_id,
                comp.quantity,
                comp.uom,
                comp.scrap_factor,
                comp.find_number,
            )
            .await?;
            tracker.record_bom_line(line_id, component_item_id, comp.quantity);
            info!(
                component = comp.component_sku,
                quantity = comp.quantity,
                line_id = %line_id,
                "BOM line created"
            );
        }

        // Step 4: Set effectivity
        set_effectivity(client, bom_url, revision_id, eff_from).await?;
        info!(revision_id = %revision_id, effective_from = %eff_from, "Effectivity set");
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_bom_definitions() {
        assert_eq!(BOMS.len(), 5, "Expected 5 BOM definitions (one per make item)");
    }

    #[test]
    fn bom_skus_are_unique() {
        let mut skus: Vec<&str> = BOMS.iter().map(|b| b.make_item_sku).collect();
        skus.sort();
        skus.dedup();
        assert_eq!(skus.len(), BOMS.len(), "Duplicate BOM make_item_sku found");
    }

    #[test]
    fn all_bom_skus_are_make_items() {
        let make_skus = [
            "TBB-ASSY-001",
            "EMB-ASSY-001",
            "SRA-ASSY-001",
            "FLC-ASSY-001",
            "LGA-ASSY-001",
        ];
        for bom in BOMS {
            assert!(
                make_skus.contains(&bom.make_item_sku),
                "BOM {} is not a known make item",
                bom.make_item_sku
            );
        }
    }

    #[test]
    fn turbine_blade_has_one_component() {
        let bom = BOMS.iter().find(|b| b.make_item_sku == "TBB-ASSY-001").unwrap();
        assert_eq!(bom.components.len(), 1);
        assert_eq!(bom.components[0].component_sku, "TI64-BAR-001");
        assert_eq!(bom.components[0].quantity, 1.0);
        assert_eq!(bom.components[0].scrap_factor, Some(0.05));
    }

    #[test]
    fn engine_mount_has_four_components() {
        let bom = BOMS.iter().find(|b| b.make_item_sku == "EMB-ASSY-001").unwrap();
        assert_eq!(bom.components.len(), 4);
        let comp_skus: Vec<&str> = bom.components.iter().map(|c| c.component_sku).collect();
        assert!(comp_skus.contains(&"AL7075-SHT-001"));
        assert!(comp_skus.contains(&"AN3-BOLT"));
        assert!(comp_skus.contains(&"MS21042-NUT"));
        assert!(comp_skus.contains(&"NAS1149-WASH"));
    }

    #[test]
    fn structural_rib_has_two_components() {
        let bom = BOMS.iter().find(|b| b.make_item_sku == "SRA-ASSY-001").unwrap();
        assert_eq!(bom.components.len(), 2);
        assert_eq!(bom.components[0].component_sku, "4130-TUB-001");
        assert_eq!(bom.components[1].component_sku, "INC718-FRG-001");
        assert_eq!(bom.components[1].scrap_factor, Some(0.03));
    }

    #[test]
    fn fuel_line_has_two_components() {
        let bom = BOMS.iter().find(|b| b.make_item_sku == "FLC-ASSY-001").unwrap();
        assert_eq!(bom.components.len(), 2);
        let comp_skus: Vec<&str> = bom.components.iter().map(|c| c.component_sku).collect();
        assert!(comp_skus.contains(&"4130-TUB-001"));
        assert!(comp_skus.contains(&"AN3-BOLT"));
    }

    #[test]
    fn landing_gear_has_two_components() {
        let bom = BOMS.iter().find(|b| b.make_item_sku == "LGA-ASSY-001").unwrap();
        assert_eq!(bom.components.len(), 2);
        let comp_skus: Vec<&str> = bom.components.iter().map(|c| c.component_sku).collect();
        assert!(comp_skus.contains(&"INC718-FRG-001"));
        assert!(comp_skus.contains(&"TI64-BAR-001"));
    }

    #[test]
    fn fasteners_only_on_assembly_types() {
        // Fasteners (AN3-BOLT, MS21042-NUT, NAS1149-WASH) should only appear
        // on Engine Mount Bracket and Fuel Line Connector, NOT on Turbine Blade
        let fastener_skus = ["AN3-BOLT", "MS21042-NUT", "NAS1149-WASH"];
        let turbine = BOMS.iter().find(|b| b.make_item_sku == "TBB-ASSY-001").unwrap();
        for comp in turbine.components {
            assert!(
                !fastener_skus.contains(&comp.component_sku),
                "Turbine blade should NOT have fastener {}",
                comp.component_sku
            );
        }
    }

    #[test]
    fn find_numbers_are_sequential_within_each_bom() {
        for bom in BOMS {
            let mut prev = 0;
            for comp in bom.components {
                assert!(
                    comp.find_number > prev,
                    "Find numbers should be ascending in BOM {}: {} <= {}",
                    bom.make_item_sku,
                    comp.find_number,
                    prev
                );
                prev = comp.find_number;
            }
        }
    }

    #[test]
    fn all_component_skus_are_in_referenced_list() {
        let ref_set: HashSet<&str> = ALL_REFERENCED_SKUS.iter().copied().collect();
        for bom in BOMS {
            assert!(
                ref_set.contains(bom.make_item_sku),
                "Make item {} not in ALL_REFERENCED_SKUS",
                bom.make_item_sku
            );
            for comp in bom.components {
                assert!(
                    ref_set.contains(comp.component_sku),
                    "Component {} not in ALL_REFERENCED_SKUS",
                    comp.component_sku
                );
            }
        }
    }

    #[test]
    fn effectivity_from_is_deterministic() {
        let d1 = effectivity_from();
        let d2 = effectivity_from();
        assert_eq!(d1, d2);
        assert_eq!(d1.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    }

    #[test]
    fn total_component_lines() {
        let total: usize = BOMS.iter().map(|b| b.components.len()).sum();
        // 1 + 4 + 2 + 2 + 2 = 11 lines total
        assert_eq!(total, 11, "Expected 11 total BOM component lines");
    }

    #[test]
    fn digest_records_boms() {
        let mut tracker = DigestTracker::new();
        let bom_id = Uuid::new_v4();
        let part_id = Uuid::new_v4();
        tracker.record_bom(bom_id, part_id);
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_records_bom_lines() {
        let mut tracker = DigestTracker::new();
        let line_id = Uuid::new_v4();
        let component_id = Uuid::new_v4();
        tracker.record_bom_line(line_id, component_id, 2.5);
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_bom_order_independent() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let part1 = Uuid::new_v4();
        let part2 = Uuid::new_v4();

        let mut t1 = DigestTracker::new();
        t1.record_bom(id1, part1);
        t1.record_bom(id2, part2);

        let mut t2 = DigestTracker::new();
        t2.record_bom(id2, part2);
        t2.record_bom(id1, part1);

        assert_eq!(
            t1.finalize(),
            t2.finalize(),
            "BOM digest should be order-independent"
        );
    }
}
