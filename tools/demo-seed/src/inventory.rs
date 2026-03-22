//! Inventory module seeder for demo-seed
//!
//! Creates units of measure, warehouse locations, and items (parts) via the
//! Inventory service API. Items reference GL accounts created by the GL seeder.
//!
//! - UoM creation: POST /api/inventory/uoms — 409 Conflict treated as success
//! - Location creation: POST /api/inventory/locations — 409 Conflict treated as success
//! - Item creation: POST /api/inventory/items — 409 Conflict triggers GET search for existing UUID

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// Deterministic warehouse UUID
// ---------------------------------------------------------------------------

/// Generate a deterministic warehouse UUID from tenant + seed.
fn warehouse_uuid(tenant: &str, seed: u64) -> Uuid {
    let name = format!("{}-warehouse-{}", tenant, seed);
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, name.as_bytes())
}

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateUomRequest {
    code: String,
    name: String,
}

#[derive(Serialize)]
struct CreateLocationRequest {
    warehouse_id: Uuid,
    code: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize)]
struct CreateItemRequest {
    sku: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uom: Option<String>,
    inventory_account_ref: String,
    cogs_account_ref: String,
    variance_account_ref: String,
    tracking_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    make_buy: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ItemResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct UomResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct LocationResponse {
    id: Uuid,
}

/// Item returned from GET /api/inventory/items search
#[derive(Debug, Deserialize)]
struct ItemSearchItem {
    id: Uuid,
    sku: String,
}

/// Search response envelope
#[derive(Debug, Deserialize)]
struct ItemSearchResponse {
    data: Vec<ItemSearchItem>,
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

struct UomDef {
    code: &'static str,
    name: &'static str,
}

const UOMS: &[UomDef] = &[
    UomDef { code: "EA", name: "Each" },
    UomDef { code: "KG", name: "Kilogram" },
    UomDef { code: "LB", name: "Pound" },
    UomDef { code: "M", name: "Meter" },
    UomDef { code: "IN", name: "Inch" },
];

struct LocationDef {
    code: &'static str,
    name: &'static str,
    description: &'static str,
}

const LOCATIONS: &[LocationDef] = &[
    LocationDef { code: "RECV-DOCK", name: "Receiving Dock", description: "Inbound material receiving area" },
    LocationDef { code: "RAW-WH", name: "Raw Material Warehouse", description: "Bulk raw material storage" },
    LocationDef { code: "WIP-FLOOR", name: "WIP Production Floor", description: "Active production work-in-progress area" },
    LocationDef { code: "FG-WH", name: "Finished Goods Warehouse", description: "Completed product storage" },
    LocationDef { code: "SHIP-DOCK", name: "Shipping Dock", description: "Outbound shipping and dispatch area" },
    LocationDef { code: "QA-HOLD", name: "Quality Hold Area", description: "Quarantine area for quality inspection" },
    LocationDef { code: "MRB", name: "Material Review Board", description: "Non-conforming material review and disposition" },
];

struct ItemDef {
    sku: &'static str,
    name: &'static str,
    description: &'static str,
    uom: &'static str,
    tracking_mode: &'static str,
    make_buy: &'static str,
    /// GL account code for inventory asset
    inventory_account: &'static str,
}

const ITEMS: &[ItemDef] = &[
    // Raw materials (buy, lot-tracked)
    ItemDef {
        sku: "TI64-BAR-001",
        name: "Ti-6Al-4V Bar Stock",
        description: "Titanium alloy bar stock, AMS 4928",
        uom: "KG",
        tracking_mode: "lot",
        make_buy: "buy",
        inventory_account: "1200",
    },
    ItemDef {
        sku: "INC718-FRG-001",
        name: "Inconel 718 Forging",
        description: "Nickel superalloy forging blank, AMS 5663",
        uom: "KG",
        tracking_mode: "lot",
        make_buy: "buy",
        inventory_account: "1200",
    },
    ItemDef {
        sku: "AL7075-SHT-001",
        name: "AL-7075-T6 Sheet",
        description: "Aluminum alloy sheet, AMS 4078",
        uom: "KG",
        tracking_mode: "lot",
        make_buy: "buy",
        inventory_account: "1200",
    },
    ItemDef {
        sku: "4130-TUB-001",
        name: "4130 Steel Tube",
        description: "Chromoly steel tubing, AMS 6345",
        uom: "M",
        tracking_mode: "lot",
        make_buy: "buy",
        inventory_account: "1200",
    },
    ItemDef {
        sku: "HXL8552-PPG-001",
        name: "Hexcel 8552 Prepreg",
        description: "Carbon fiber epoxy prepreg, HexPly 8552",
        uom: "KG",
        tracking_mode: "lot",
        make_buy: "buy",
        inventory_account: "1200",
    },
    // Manufactured parts (make, lot-tracked)
    ItemDef {
        sku: "TBB-ASSY-001",
        name: "Turbine Blade Blank",
        description: "Investment-cast turbine blade blank, Stage 1 HPT",
        uom: "EA",
        tracking_mode: "lot",
        make_buy: "make",
        inventory_account: "1220",
    },
    ItemDef {
        sku: "EMB-ASSY-001",
        name: "Engine Mount Bracket",
        description: "CNC-machined engine mount bracket assembly",
        uom: "EA",
        tracking_mode: "lot",
        make_buy: "make",
        inventory_account: "1220",
    },
    ItemDef {
        sku: "SRA-ASSY-001",
        name: "Structural Rib Assembly",
        description: "Wing structural rib, multi-piece riveted assembly",
        uom: "EA",
        tracking_mode: "lot",
        make_buy: "make",
        inventory_account: "1220",
    },
    ItemDef {
        sku: "FLC-ASSY-001",
        name: "Fuel Line Connector",
        description: "Precision fuel line connector, AN-style fitting",
        uom: "EA",
        tracking_mode: "lot",
        make_buy: "make",
        inventory_account: "1220",
    },
    ItemDef {
        sku: "LGA-ASSY-001",
        name: "Landing Gear Actuator Housing",
        description: "Forged and machined hydraulic actuator housing",
        uom: "EA",
        tracking_mode: "lot",
        make_buy: "make",
        inventory_account: "1220",
    },
    // Fasteners (buy, no tracking)
    ItemDef {
        sku: "AN3-BOLT",
        name: "AN3 Bolt",
        description: "AN3 standard hex bolt, cadmium plated",
        uom: "EA",
        tracking_mode: "none",
        make_buy: "buy",
        inventory_account: "1200",
    },
    ItemDef {
        sku: "MS21042-NUT",
        name: "MS21042 Nut",
        description: "MS21042 self-locking nut, steel",
        uom: "EA",
        tracking_mode: "none",
        make_buy: "buy",
        inventory_account: "1200",
    },
    ItemDef {
        sku: "NAS1149-WASH",
        name: "NAS1149 Washer",
        description: "NAS1149 flat washer, corrosion resistant steel",
        uom: "EA",
        tracking_mode: "none",
        make_buy: "buy",
        inventory_account: "1200",
    },
];

/// COGS account for all items
const COGS_ACCOUNT_REF: &str = "5000";
/// Purchase price variance account for all items
const VARIANCE_ACCOUNT_REF: &str = "5100";

// ---------------------------------------------------------------------------
// Public return type
// ---------------------------------------------------------------------------

/// Created inventory resource IDs for downstream modules (BOM, production)
pub struct InventoryIds {
    pub warehouse_id: Uuid,
    pub items: Vec<(Uuid, String, String)>, // (id, sku, make_buy)
    pub locations: Vec<(Uuid, String)>,     // (id, code)
    pub uom_count: usize,
}

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

async fn create_uom(
    client: &reqwest::Client,
    inventory_url: &str,
    uom: &UomDef,
) -> Result<Option<Uuid>> {
    let url = format!("{}/api/inventory/uoms", inventory_url);

    let body = CreateUomRequest {
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
        info!(code = uom.code, "UoM already exists");
        return Ok(None);
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

async fn create_location(
    client: &reqwest::Client,
    inventory_url: &str,
    wh_id: Uuid,
    loc: &LocationDef,
) -> Result<Option<Uuid>> {
    let url = format!("{}/api/inventory/locations", inventory_url);

    let body = CreateLocationRequest {
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

async fn create_item(
    client: &reqwest::Client,
    inventory_url: &str,
    item: &ItemDef,
) -> Result<Uuid> {
    let url = format!("{}/api/inventory/items", inventory_url);

    let body = CreateItemRequest {
        sku: item.sku.to_string(),
        name: item.name.to_string(),
        description: Some(item.description.to_string()),
        uom: Some(item.uom.to_string()),
        inventory_account_ref: item.inventory_account.to_string(),
        cogs_account_ref: COGS_ACCOUNT_REF.to_string(),
        variance_account_ref: VARIANCE_ACCOUNT_REF.to_string(),
        tracking_mode: item.tracking_mode.to_string(),
        make_buy: Some(item.make_buy.to_string()),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/inventory/items ({}) network error", item.sku))?;

    let status = resp.status();

    if status == reqwest::StatusCode::CONFLICT {
        // Item already exists — retrieve UUID via search
        info!(sku = item.sku, "Item already exists — retrieving UUID via search");
        return find_item_by_sku(client, inventory_url, item.sku).await;
    }

    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let item_resp: ItemResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse item response for {}", item.sku))?;
        return Ok(item_resp.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/inventory/items ({}) failed {status}: {text}",
        item.sku
    );
}

/// Search for an existing item by exact SKU match.
/// The API uses ILIKE substring matching, so we filter client-side.
async fn find_item_by_sku(
    client: &reqwest::Client,
    inventory_url: &str,
    sku: &str,
) -> Result<Uuid> {
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
        bail!("GET /api/inventory/items?search={} failed {status}: {text}", sku);
    }

    let search: ItemSearchResponse = resp
        .json()
        .await
        .with_context(|| format!("Failed to parse item search response for {}", sku))?;

    // Exact match on SKU (API uses ILIKE substring match)
    for item in &search.data {
        if item.sku == sku {
            return Ok(item.id);
        }
    }

    bail!(
        "Item with SKU '{}' returned 409 but could not be found via search",
        sku
    );
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Seed inventory data (UoMs, locations, items). Returns IDs for downstream modules.
pub async fn seed_inventory(
    client: &reqwest::Client,
    inventory_url: &str,
    tenant: &str,
    seed: u64,
    tracker: &mut DigestTracker,
) -> Result<InventoryIds> {
    let wh_id = warehouse_uuid(tenant, seed);
    info!(warehouse_id = %wh_id, "Using deterministic warehouse UUID");

    // --- UoMs ---
    let mut uom_count = 0;
    for uom in UOMS {
        let maybe_id = create_uom(client, inventory_url, uom).await?;
        if let Some(id) = maybe_id {
            tracker.record_uom(id, uom.code);
        } else {
            // 409 — still record for digest determinism using a deterministic placeholder
            let placeholder = Uuid::new_v5(&Uuid::NAMESPACE_DNS, format!("uom-{}", uom.code).as_bytes());
            tracker.record_uom(placeholder, uom.code);
        }
        uom_count += 1;
        info!(code = uom.code, name = uom.name, "UoM seeded");
    }

    // --- Locations ---
    let mut locations = Vec::with_capacity(LOCATIONS.len());
    for loc in LOCATIONS {
        let maybe_id = create_location(client, inventory_url, wh_id, loc).await?;
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

    // --- Items ---
    let mut items = Vec::with_capacity(ITEMS.len());
    for item in ITEMS {
        let item_id = create_item(client, inventory_url, item).await?;
        tracker.record_item(item_id, item.sku, item.make_buy);
        items.push((item_id, item.sku.to_string(), item.make_buy.to_string()));
        info!(
            sku = item.sku,
            name = item.name,
            item_id = %item_id,
            make_buy = item.make_buy,
            tracking_mode = item.tracking_mode,
            "Item seeded"
        );
    }

    Ok(InventoryIds {
        warehouse_id: wh_id,
        items,
        locations,
        uom_count,
    })
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
    fn seven_locations_defined() {
        assert_eq!(LOCATIONS.len(), 7, "Expected 7 locations");
    }

    #[test]
    fn thirteen_items_defined() {
        assert_eq!(ITEMS.len(), 13, "Expected 13 items");
    }

    #[test]
    fn uom_codes_are_unique() {
        let mut codes: Vec<&str> = UOMS.iter().map(|u| u.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), UOMS.len(), "Duplicate UoM codes found");
    }

    #[test]
    fn location_codes_are_unique() {
        let mut codes: Vec<&str> = LOCATIONS.iter().map(|l| l.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), LOCATIONS.len(), "Duplicate location codes found");
    }

    #[test]
    fn item_skus_are_unique() {
        let mut skus: Vec<&str> = ITEMS.iter().map(|i| i.sku).collect();
        skus.sort();
        skus.dedup();
        assert_eq!(skus.len(), ITEMS.len(), "Duplicate item SKUs found");
    }

    #[test]
    fn five_raw_materials() {
        let raw: Vec<_> = ITEMS
            .iter()
            .filter(|i| i.make_buy == "buy" && i.tracking_mode == "lot")
            .collect();
        assert_eq!(raw.len(), 5, "Expected 5 raw materials (buy + lot-tracked)");
    }

    #[test]
    fn five_manufactured_parts() {
        let made: Vec<_> = ITEMS.iter().filter(|i| i.make_buy == "make").collect();
        assert_eq!(made.len(), 5, "Expected 5 manufactured parts");
    }

    #[test]
    fn three_fasteners() {
        let fasteners: Vec<_> = ITEMS
            .iter()
            .filter(|i| i.make_buy == "buy" && i.tracking_mode == "none")
            .collect();
        assert_eq!(fasteners.len(), 3, "Expected 3 fasteners (buy + no tracking)");
    }

    #[test]
    fn make_items_use_finished_goods_account() {
        for item in ITEMS.iter().filter(|i| i.make_buy == "make") {
            assert_eq!(
                item.inventory_account, "1220",
                "Make item {} should use 1220 (Finished Goods)",
                item.sku
            );
        }
    }

    #[test]
    fn buy_items_use_raw_materials_account() {
        for item in ITEMS.iter().filter(|i| i.make_buy == "buy") {
            assert_eq!(
                item.inventory_account, "1200",
                "Buy item {} should use 1200 (Raw Materials)",
                item.sku
            );
        }
    }

    #[test]
    fn all_items_use_correct_cogs_and_variance() {
        for item in ITEMS {
            assert_eq!(COGS_ACCOUNT_REF, "5000", "COGS should be 5000");
            assert_eq!(VARIANCE_ACCOUNT_REF, "5100", "Variance should be 5100");
            // These are constants, but verify the item struct would use them
            assert!(
                !item.sku.is_empty(),
                "Item must have a SKU to reference GL accounts"
            );
        }
    }

    #[test]
    fn manufactured_parts_are_lot_tracked() {
        for item in ITEMS.iter().filter(|i| i.make_buy == "make") {
            assert_eq!(
                item.tracking_mode, "lot",
                "Manufactured item {} should be lot-tracked",
                item.sku
            );
        }
    }

    #[test]
    fn fasteners_have_no_tracking() {
        let fastener_skus = ["AN3-BOLT", "MS21042-NUT", "NAS1149-WASH"];
        for sku in &fastener_skus {
            let item = ITEMS.iter().find(|i| i.sku == *sku).unwrap();
            assert_eq!(
                item.tracking_mode, "none",
                "Fastener {} should have tracking_mode=none",
                sku
            );
        }
    }

    #[test]
    fn warehouse_uuid_is_deterministic() {
        let id1 = warehouse_uuid("t1", 42);
        let id2 = warehouse_uuid("t1", 42);
        assert_eq!(id1, id2, "Same tenant+seed should produce same warehouse UUID");
    }

    #[test]
    fn warehouse_uuid_differs_by_tenant() {
        let id1 = warehouse_uuid("t1", 42);
        let id2 = warehouse_uuid("t2", 42);
        assert_ne!(id1, id2, "Different tenants should produce different warehouse UUIDs");
    }

    #[test]
    fn warehouse_uuid_differs_by_seed() {
        let id1 = warehouse_uuid("t1", 42);
        let id2 = warehouse_uuid("t1", 99);
        assert_ne!(id1, id2, "Different seeds should produce different warehouse UUIDs");
    }

    #[test]
    fn tracking_modes_are_valid() {
        let valid = ["lot", "serial", "none"];
        for item in ITEMS {
            assert!(
                valid.contains(&item.tracking_mode),
                "Item {} has invalid tracking_mode: {}",
                item.sku,
                item.tracking_mode
            );
        }
    }

    #[test]
    fn make_buy_values_are_valid() {
        let valid = ["make", "buy"];
        for item in ITEMS {
            assert!(
                valid.contains(&item.make_buy),
                "Item {} has invalid make_buy: {}",
                item.sku,
                item.make_buy
            );
        }
    }

    #[test]
    fn all_items_have_descriptions() {
        for item in ITEMS {
            assert!(
                !item.description.is_empty(),
                "Item {} missing description",
                item.sku
            );
        }
    }

    #[test]
    fn digest_records_items() {
        let mut tracker = DigestTracker::new();
        let id = Uuid::new_v4();
        tracker.record_item(id, "TI64-BAR-001", "buy");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_records_locations() {
        let mut tracker = DigestTracker::new();
        let id = Uuid::new_v4();
        tracker.record_location(id, "RECV-DOCK");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
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
