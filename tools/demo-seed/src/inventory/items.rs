//! Inventory item seeding for demo-seed

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

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

pub(super) struct ItemDef {
    pub sku: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub uom: &'static str,
    pub tracking_mode: &'static str,
    pub make_buy: &'static str,
    /// GL account code for inventory asset
    pub inventory_account: &'static str,
}

pub(super) const ITEMS: &[ItemDef] = &[
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
// HTTP operations
// ---------------------------------------------------------------------------

pub(super) async fn create_item(
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
// Seeding logic
// ---------------------------------------------------------------------------

pub(super) async fn seed_items(
    client: &reqwest::Client,
    inventory_url: &str,
    tracker: &mut DigestTracker,
) -> Result<Vec<(Uuid, String, String)>> {
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
    Ok(items)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirteen_items_defined() {
        assert_eq!(ITEMS.len(), 13, "Expected 13 items");
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
}
