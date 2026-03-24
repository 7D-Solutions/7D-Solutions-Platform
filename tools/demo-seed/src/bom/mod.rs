//! BOM (Bill of Materials) module seeder for demo-seed
//!
//! Creates bills of materials with revisions and component lines for
//! aerospace manufacturing items. Depends on inventory items existing first.
//!
//! Uses GET-before-create pattern for idempotent re-runs:
//! 1. GET /api/bom/by-part/{part_id} to check if BOM exists
//! 2. GET /api/bom/{bom_id}/revisions to check if revision A exists
//! 3. GET /api/bom/revisions/{revision_id}/lines to check existing lines

mod data;
mod headers;
mod lines;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;
use data::{ALL_REFERENCED_SKUS, BOMS};
use headers::{effectivity_from, get_or_create_bom, get_or_create_revision, set_effectivity};
use lines::{create_line, get_revision_lines};

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
// Inventory item resolution (for standalone --modules bom)
// ---------------------------------------------------------------------------

// Inventory search types (for standalone --modules bom)

#[derive(Debug, Deserialize)]
struct InventorySearchItem {
    id: Uuid,
    sku: String,
}

#[derive(Debug, Deserialize)]
struct InventorySearchResponse {
    items: Vec<InventorySearchItem>,
}

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
            .items
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
        tracker.record_bom(bom_id, bom_def.make_item_sku);
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
                tracker.record_bom_line(existing.id, comp.component_sku, comp.quantity);
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
            tracker.record_bom_line(line_id, comp.component_sku, comp.quantity);
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
    fn total_component_lines() {
        let total: usize = BOMS.iter().map(|b| b.components.len()).sum();
        // 1 + 4 + 2 + 2 + 2 = 11 lines total
        assert_eq!(total, 11, "Expected 11 total BOM component lines");
    }

    #[test]
    fn digest_records_boms() {
        let mut tracker = DigestTracker::new();
        let bom_id = Uuid::new_v4();
        tracker.record_bom(bom_id, "TBB-ASSY-001");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_records_bom_lines() {
        let mut tracker = DigestTracker::new();
        let line_id = Uuid::new_v4();
        tracker.record_bom_line(line_id, "TI64-BAR-001", 2.5);
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_bom_order_independent() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let mut t1 = DigestTracker::new();
        t1.record_bom(id1, "TBB-ASSY-001");
        t1.record_bom(id2, "EMB-ASSY-001");

        let mut t2 = DigestTracker::new();
        t2.record_bom(id2, "EMB-ASSY-001");
        t2.record_bom(id1, "TBB-ASSY-001");

        assert_eq!(
            t1.finalize(),
            t2.finalize(),
            "BOM digest should be order-independent"
        );
    }
}
