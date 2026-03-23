//! Inventory module seeder for demo-seed
//!
//! Creates units of measure, warehouse locations, and items (parts) via the
//! Inventory service API. Items reference GL accounts created by the GL seeder.
//!
//! - UoM creation: POST /api/inventory/uoms — 409 Conflict treated as success
//! - Location creation: POST /api/inventory/locations — 409 Conflict treated as success
//! - Item creation: POST /api/inventory/items — 409 Conflict triggers GET search for existing UUID

mod items;
mod locations;
mod uoms;

use anyhow::Result;
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
// Public return type
// ---------------------------------------------------------------------------

/// Created inventory resource IDs for downstream modules (BOM, production)
pub struct InventoryIds {
    pub warehouse_id: Uuid,
    pub items: Vec<(Uuid, String, String)>, // (id, sku, make_buy)
    pub locations: Vec<(Uuid, String)>,     // (id, code)
    pub uoms: Vec<(Uuid, String)>,          // (id, code)
    pub uom_count: usize,
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
    let (uoms, uom_count) = uoms::seed_uoms(client, inventory_url, tenant, tracker).await?;

    // --- Locations ---
    let locations = locations::seed_locations(client, inventory_url, tenant, wh_id, tracker).await?;

    // --- Items ---
    let items = items::seed_items(client, inventory_url, tenant, tracker).await?;

    Ok(InventoryIds {
        warehouse_id: wh_id,
        items,
        locations,
        uoms,
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
}
