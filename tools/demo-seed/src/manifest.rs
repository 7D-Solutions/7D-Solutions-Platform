//! JSON manifest output for demo-seed
//!
//! Produces a machine-readable manifest of all created resource IDs.
//! Downstream systems (e.g. Fireproof) consume this to reference
//! Platform entities in their own seed scripts.

use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

use crate::bom::BomIds;
use crate::gl::GlAccounts;
use crate::inventory::InventoryIds;
use crate::party::PartyIds;
use crate::production::ProductionIds;

// ---------------------------------------------------------------------------
// Manifest schema types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub tenant_id: String,
    pub seed: u64,
    pub digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<ManifestUsers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering: Option<ManifestNumbering>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gl: Option<ManifestGl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parties: Option<ManifestParties>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<ManifestInventory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bom: Option<ManifestBom>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub production: Option<ManifestProduction>,
}

#[derive(Debug, Serialize)]
pub struct ManifestUsers {
    pub admin: ManifestUser,
}

#[derive(Debug, Serialize)]
pub struct ManifestUser {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestNumbering {
    pub policies: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ManifestGl {
    pub accounts: Vec<ManifestGlAccount>,
    pub fx_rates: Vec<ManifestFxRate>,
}

#[derive(Debug, Serialize)]
pub struct ManifestGlAccount {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestFxRate {
    pub rate_id: Uuid,
    pub pair: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestParties {
    pub customers: Vec<ManifestParty>,
    pub suppliers: Vec<ManifestParty>,
}

#[derive(Debug, Serialize)]
pub struct ManifestParty {
    pub id: Uuid,
    pub legal_name: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestInventory {
    pub items: Vec<ManifestItem>,
    pub locations: Vec<ManifestLocation>,
    pub uoms: Vec<ManifestUom>,
    pub warehouse_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ManifestItem {
    pub id: Uuid,
    pub sku: String,
    pub make_buy: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestLocation {
    pub id: Uuid,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestUom {
    pub id: Uuid,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestBomEntry {
    pub id: Uuid,
    pub part_id: Uuid,
    pub revision_id: Uuid,
    pub revision_label: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestBom {
    pub boms: Vec<ManifestBomEntry>,
}

#[derive(Debug, Serialize)]
pub struct ManifestProduction {
    pub workcenters: Vec<ManifestWorkcenter>,
    pub routings: Vec<ManifestRouting>,
}

#[derive(Debug, Serialize)]
pub struct ManifestWorkcenter {
    pub id: Uuid,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestRouting {
    pub id: Uuid,
    pub item_id: Uuid,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builds the manifest from module results.
pub struct ManifestBuilder {
    tenant_id: String,
    seed: u64,
    digest: String,
    numbering_policies: Option<Vec<String>>,
    gl: Option<GlAccounts>,
    parties: Option<PartyIds>,
    inventory: Option<InventoryIds>,
    bom: Option<BomIds>,
    production: Option<ProductionIds>,
}

impl ManifestBuilder {
    pub fn new(tenant_id: String, seed: u64, digest: String) -> Self {
        Self {
            tenant_id,
            seed,
            digest,
            numbering_policies: None,
            gl: None,
            parties: None,
            inventory: None,
            bom: None,
            production: None,
        }
    }

    pub fn with_numbering(mut self, policies: Vec<String>) -> Self {
        self.numbering_policies = Some(policies);
        self
    }

    pub fn with_gl(mut self, gl: GlAccounts) -> Self {
        self.gl = Some(gl);
        self
    }

    pub fn with_parties(mut self, parties: PartyIds) -> Self {
        self.parties = Some(parties);
        self
    }

    pub fn with_inventory(mut self, inv: InventoryIds) -> Self {
        self.inventory = Some(inv);
        self
    }

    pub fn with_bom(mut self, bom: BomIds) -> Self {
        self.bom = Some(bom);
        self
    }

    pub fn with_production(mut self, prod: ProductionIds) -> Self {
        self.production = Some(prod);
        self
    }

    pub fn build(self) -> Manifest {
        let users = Some(ManifestUsers {
            admin: ManifestUser {
                id: None,
                email: "admin@7dsolutions.local".to_string(),
            },
        });

        let numbering = self.numbering_policies.map(|p| ManifestNumbering { policies: p });

        let gl = self.gl.map(|g| ManifestGl {
            accounts: g.accounts.into_iter().map(|(code, name)| ManifestGlAccount { code, name }).collect(),
            fx_rates: g.fx_rates.into_iter().map(|(rate_id, pair)| ManifestFxRate { rate_id, pair }).collect(),
        });

        let parties = self.parties.map(|p| ManifestParties {
            customers: p.customers.into_iter().map(|(id, legal_name)| ManifestParty { id, legal_name }).collect(),
            suppliers: p.suppliers.into_iter().map(|(id, legal_name)| ManifestParty { id, legal_name }).collect(),
        });

        let inventory = self.inventory.map(|inv| ManifestInventory {
            warehouse_id: inv.warehouse_id,
            items: inv.items.into_iter().map(|(id, sku, make_buy)| ManifestItem { id, sku, make_buy }).collect(),
            locations: inv.locations.into_iter().map(|(id, code)| ManifestLocation { id, code }).collect(),
            uoms: inv.uoms.into_iter().map(|(id, code)| ManifestUom { id, code }).collect(),
        });

        let bom = self.bom.map(|b| {
            let boms = b.boms.iter().zip(b.revisions.iter()).map(|((bom_id, part_id, _sku), (rev_id, _bom_id))| {
                ManifestBomEntry {
                    id: *bom_id,
                    part_id: *part_id,
                    revision_id: *rev_id,
                    revision_label: "A".to_string(),
                }
            }).collect();
            ManifestBom { boms }
        });

        let production = self.production.map(|p| ManifestProduction {
            workcenters: p.workcenter_list.into_iter().map(|(id, code)| ManifestWorkcenter { id, code }).collect(),
            routings: p.routing_list.into_iter().map(|(id, item_id)| ManifestRouting { id, item_id }).collect(),
        });

        Manifest {
            tenant_id: self.tenant_id,
            seed: self.seed,
            digest: self.digest,
            users,
            numbering,
            gl,
            parties,
            inventory,
            bom,
            production,
        }
    }
}

/// Write manifest to file or return as JSON string.
pub fn write_manifest(manifest: &Manifest, path: Option<&Path>) -> anyhow::Result<String> {
    let json = serde_json::to_string_pretty(manifest)?;
    if let Some(p) = path {
        std::fs::write(p, &json)?;
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manifest_serializes() {
        let m = ManifestBuilder::new("t1".into(), 42, "abc123".into()).build();
        let json = serde_json::to_string_pretty(&m).unwrap();
        assert!(json.contains("\"tenant_id\": \"t1\""));
        assert!(json.contains("\"seed\": 42"));
    }

    #[test]
    fn manifest_with_numbering() {
        let m = ManifestBuilder::new("t1".into(), 42, "abc".into())
            .with_numbering(vec!["purchase-order".into(), "sales-order".into()])
            .build();
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("purchase-order"));
        assert!(json.contains("sales-order"));
    }

    #[test]
    fn manifest_skips_none_sections() {
        let m = ManifestBuilder::new("t1".into(), 42, "abc".into()).build();
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("\"gl\""));
        assert!(!json.contains("\"inventory\""));
    }
}
