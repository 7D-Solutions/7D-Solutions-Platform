//! Production module seeder for demo-seed
//!
//! Creates work centers and routing templates with steps for aerospace
//! manufacturing via the Production service API.
//!
//! - Work center creation: POST /api/production/workcenters — 409 on duplicate code
//! - Routing creation: POST /api/production/routings — 409 on duplicate (item_id, revision)
//! - Routing step creation: POST /api/production/routings/{id}/steps — 409 on duplicate sequence

mod data;
pub(crate) mod workcenters;
mod routings;

use std::collections::HashMap;

use anyhow::Result;
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;
use routings::{add_routing_step, create_routing, ROUTINGS};
use workcenters::{create_workcenter, WORKCENTERS};

/// Summary of created production resources
pub struct ProductionIds {
    pub workcenters: usize,
    pub routings: usize,
    pub workcenter_list: Vec<(Uuid, String)>,  // (id, code)
    pub routing_list: Vec<(Uuid, Uuid)>,       // (routing_id, item_id)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Seed production data (work centers, routings, routing steps).
///
/// `item_ids` maps item SKU → item UUID (from inventory seeding).
pub async fn seed_production(
    client: &reqwest::Client,
    production_url: &str,
    tenant: &str,
    item_ids: &HashMap<String, Uuid>,
    tracker: &mut DigestTracker,
) -> Result<ProductionIds> {
    // --- Work centers (must be created first — routing steps reference them) ---
    let mut wc_map: HashMap<String, Uuid> = HashMap::new();
    let mut workcenter_list = Vec::with_capacity(WORKCENTERS.len());

    for wc in WORKCENTERS {
        let wc_id = create_workcenter(client, production_url, tenant, wc).await?;
        tracker.record_workcenter(wc_id, wc.code);
        wc_map.insert(wc.code.to_string(), wc_id);
        workcenter_list.push((wc_id, wc.code.to_string()));
        info!(
            code = wc.code,
            name = wc.name,
            workcenter_id = %wc_id,
            cost_rate_minor = wc.cost_rate_minor,
            "Workcenter seeded"
        );
    }

    // --- Routings and steps ---
    let mut routing_count = 0;
    let mut routing_list = Vec::with_capacity(ROUTINGS.len());

    for routing in ROUTINGS {
        let item_id = item_ids.get(routing.item_sku).ok_or_else(|| {
            anyhow::anyhow!(
                "Item SKU '{}' not found in inventory IDs — was inventory seeded first?",
                routing.item_sku
            )
        })?;

        let routing_id = create_routing(client, production_url, tenant, routing, *item_id).await?;
        tracker.record_routing(routing_id, routing.item_sku, "1");
        routing_list.push((routing_id, *item_id));

        // Add steps in manufacturing sequence order
        for (idx, step) in routing.steps.iter().enumerate() {
            let wc_id = wc_map.get(step.workcenter_code).ok_or_else(|| {
                anyhow::anyhow!(
                    "Workcenter code '{}' not found — check WORKCENTERS definition",
                    step.workcenter_code
                )
            })?;

            let sequence = (idx as i32) + 1; // 1-based
            add_routing_step(client, production_url, tenant, routing_id, sequence, *wc_id, step).await?;

            info!(
                routing_id = %routing_id,
                item_sku = routing.item_sku,
                sequence,
                operation = step.operation_name,
                setup_min = step.setup_time_minutes,
                run_min = step.run_time_minutes,
                "Routing step added"
            );
        }

        routing_count += 1;
        info!(
            item_sku = routing.item_sku,
            routing_id = %routing_id,
            steps = routing.steps.len(),
            "Routing seeded"
        );
    }

    Ok(ProductionIds {
        workcenters: wc_map.len(),
        routings: routing_count,
        workcenter_list,
        routing_list,
    })
}
