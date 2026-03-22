//! Production module seeder for demo-seed
//!
//! Creates work centers and routing templates with steps for aerospace
//! manufacturing via the Production service API.
//!
//! - Work center creation: POST /api/production/workcenters — 409 on duplicate code
//! - Routing creation: POST /api/production/routings — 409 on duplicate (item_id, revision)
//! - Routing step creation: POST /api/production/routings/{id}/steps — 409 on duplicate sequence

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateWorkcenterRequest {
    code: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capacity: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_rate_minor: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct WorkcenterResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct WorkcenterListItem {
    id: Uuid,
    code: String,
}

#[derive(Serialize)]
struct CreateRoutingRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoutingResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct RoutingByItemEntry {
    id: Uuid,
}

#[derive(Serialize)]
struct AddRoutingStepRequest {
    sequence_number: i32,
    workcenter_id: Uuid,
    operation_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_time_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_time_minutes: Option<i32>,
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

struct WorkcenterDef {
    code: &'static str,
    name: &'static str,
    description: &'static str,
    capacity: i32,
    cost_rate_minor: i64,
}

const WORKCENTERS: &[WorkcenterDef] = &[
    WorkcenterDef {
        code: "CNC-MILL-01",
        name: "Haas VF-3 CNC Mill",
        description: "5-axis CNC vertical machining center",
        capacity: 1,
        cost_rate_minor: 15000,
    },
    WorkcenterDef {
        code: "CNC-LATHE-01",
        name: "Mazak QTN-200 CNC Lathe",
        description: "CNC turning center with live tooling",
        capacity: 1,
        cost_rate_minor: 12000,
    },
    WorkcenterDef {
        code: "HEAT-TREAT",
        name: "Bodycote Heat Treat Furnace",
        description: "Vacuum heat treatment furnace",
        capacity: 4,
        cost_rate_minor: 8000,
    },
    WorkcenterDef {
        code: "GRIND-01",
        name: "Studer S33 Cylindrical Grinder",
        description: "CNC cylindrical grinding machine",
        capacity: 1,
        cost_rate_minor: 10000,
    },
    WorkcenterDef {
        code: "ASSEMBLY-01",
        name: "Manual Assembly Station",
        description: "Ergonomic assembly workstation with torque tools",
        capacity: 2,
        cost_rate_minor: 5000,
    },
    WorkcenterDef {
        code: "NDT-01",
        name: "NDT Inspection Station",
        description: "Non-destructive testing station (UT, FPI, X-ray)",
        capacity: 1,
        cost_rate_minor: 20000,
    },
];

struct RoutingStepDef {
    /// Work center code (resolved to UUID at runtime)
    workcenter_code: &'static str,
    operation_name: &'static str,
    description: &'static str,
    setup_time_minutes: i32,
    run_time_minutes: i32,
}

struct RoutingDef {
    /// SKU of the make item (resolved to UUID at runtime)
    item_sku: &'static str,
    name: &'static str,
    description: &'static str,
    steps: &'static [RoutingStepDef],
}

const ROUTINGS: &[RoutingDef] = &[
    RoutingDef {
        item_sku: "TBB-ASSY-001",
        name: "Turbine Blade Blank Routing",
        description: "5-step HPT blade blank: rough mill → finish mill → heat treat → grind → NDT",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "Rough Mill",
                description: "Rough CNC milling of blade blank profile",
                setup_time_minutes: 30,
                run_time_minutes: 45,
            },
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "Finish Mill",
                description: "Finish CNC milling to final blade profile tolerance",
                setup_time_minutes: 15,
                run_time_minutes: 60,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Solution heat treat and age per AMS 4928",
                setup_time_minutes: 10,
                run_time_minutes: 480,
            },
            RoutingStepDef {
                workcenter_code: "GRIND-01",
                operation_name: "Cylindrical Grind",
                description: "Finish grind root and platform surfaces",
                setup_time_minutes: 15,
                run_time_minutes: 30,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "FPI and UT inspection per ASTM E1444",
                setup_time_minutes: 5,
                run_time_minutes: 20,
            },
        ],
    },
    RoutingDef {
        item_sku: "EMB-ASSY-001",
        name: "Engine Mount Bracket Routing",
        description: "3-step bracket: CNC mill → heat treat → NDT",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "CNC Mill",
                description: "Machine bracket from billet stock",
                setup_time_minutes: 20,
                run_time_minutes: 35,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Stress relieve and age harden",
                setup_time_minutes: 10,
                run_time_minutes: 360,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "Magnetic particle inspection per ASTM E1444",
                setup_time_minutes: 5,
                run_time_minutes: 15,
            },
        ],
    },
    RoutingDef {
        item_sku: "SRA-ASSY-001",
        name: "Structural Rib Assembly Routing",
        description: "4-step rib: CNC mill → lathe → heat treat → assembly",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "CNC Mill",
                description: "Machine rib web and flanges from plate",
                setup_time_minutes: 25,
                run_time_minutes: 50,
            },
            RoutingStepDef {
                workcenter_code: "CNC-LATHE-01",
                operation_name: "CNC Lathe",
                description: "Turn bushings and sleeve inserts",
                setup_time_minutes: 15,
                run_time_minutes: 30,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Precipitation hardening cycle",
                setup_time_minutes: 10,
                run_time_minutes: 240,
            },
            RoutingStepDef {
                workcenter_code: "ASSEMBLY-01",
                operation_name: "Assembly",
                description: "Rivet and assemble rib components per drawing",
                setup_time_minutes: 10,
                run_time_minutes: 45,
            },
        ],
    },
    RoutingDef {
        item_sku: "FLC-ASSY-001",
        name: "Fuel Line Connector Routing",
        description: "3-step connector: lathe → grind → NDT",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-LATHE-01",
                operation_name: "CNC Lathe",
                description: "Turn connector body and thread AN fittings",
                setup_time_minutes: 15,
                run_time_minutes: 20,
            },
            RoutingStepDef {
                workcenter_code: "GRIND-01",
                operation_name: "Cylindrical Grind",
                description: "Finish grind sealing surfaces",
                setup_time_minutes: 10,
                run_time_minutes: 15,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "Dye penetrant inspection of sealing surfaces",
                setup_time_minutes: 5,
                run_time_minutes: 10,
            },
        ],
    },
    RoutingDef {
        item_sku: "LGA-ASSY-001",
        name: "Landing Gear Actuator Housing Routing",
        description: "5-step housing: CNC mill → heat treat → grind → NDT → assembly",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "CNC Mill",
                description: "Machine housing bore and mounting faces from forging",
                setup_time_minutes: 30,
                run_time_minutes: 90,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Full heat treatment cycle per material spec",
                setup_time_minutes: 10,
                run_time_minutes: 480,
            },
            RoutingStepDef {
                workcenter_code: "GRIND-01",
                operation_name: "Cylindrical Grind",
                description: "Precision grind bore and piston surfaces",
                setup_time_minutes: 15,
                run_time_minutes: 45,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "Ultrasonic and FPI inspection of critical surfaces",
                setup_time_minutes: 5,
                run_time_minutes: 30,
            },
            RoutingStepDef {
                workcenter_code: "ASSEMBLY-01",
                operation_name: "Final Assembly",
                description: "Install seals, bushings, and hydraulic ports",
                setup_time_minutes: 15,
                run_time_minutes: 60,
            },
        ],
    },
];

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

/// Create a work center; on 409, list all and find by code.
async fn create_workcenter(
    client: &reqwest::Client,
    production_url: &str,
    wc: &WorkcenterDef,
) -> Result<Uuid> {
    let url = format!("{}/api/production/workcenters", production_url);

    let body = CreateWorkcenterRequest {
        code: wc.code.to_string(),
        name: wc.name.to_string(),
        description: Some(wc.description.to_string()),
        capacity: Some(wc.capacity),
        cost_rate_minor: Some(wc.cost_rate_minor),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/production/workcenters ({}) network error", wc.code))?;

    let status = resp.status();

    if status == reqwest::StatusCode::CONFLICT {
        info!(code = wc.code, "Workcenter already exists — retrieving UUID via list");
        return find_workcenter_by_code(client, production_url, wc.code).await;
    }

    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let wc_resp: WorkcenterResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse workcenter response for {}", wc.code))?;
        return Ok(wc_resp.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/production/workcenters ({}) failed {status}: {text}",
        wc.code
    );
}

/// List all work centers and find by code.
async fn find_workcenter_by_code(
    client: &reqwest::Client,
    production_url: &str,
    code: &str,
) -> Result<Uuid> {
    let url = format!("{}/api/production/workcenters", production_url);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| "GET /api/production/workcenters network error".to_string())?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("GET /api/production/workcenters failed {status}: {text}");
    }

    let items: Vec<WorkcenterListItem> = resp
        .json()
        .await
        .context("Failed to parse workcenter list response")?;

    for wc in &items {
        if wc.code == code {
            return Ok(wc.id);
        }
    }

    bail!(
        "Workcenter with code '{}' returned 409 but could not be found in list",
        code
    );
}

/// Create a routing; on 409, find by item_id.
async fn create_routing(
    client: &reqwest::Client,
    production_url: &str,
    routing: &RoutingDef,
    item_id: Uuid,
) -> Result<Uuid> {
    let url = format!("{}/api/production/routings", production_url);

    let body = CreateRoutingRequest {
        name: routing.name.to_string(),
        description: Some(routing.description.to_string()),
        item_id: Some(item_id),
        revision: Some("1".to_string()),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| {
            format!(
                "POST /api/production/routings ({}) network error",
                routing.item_sku
            )
        })?;

    let status = resp.status();

    if status == reqwest::StatusCode::CONFLICT {
        info!(
            item_sku = routing.item_sku,
            "Routing already exists — retrieving UUID by item_id"
        );
        return find_routing_by_item(client, production_url, item_id).await;
    }

    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let rt_resp: RoutingResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse routing response for {}", routing.item_sku))?;
        return Ok(rt_resp.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/production/routings ({}) failed {status}: {text}",
        routing.item_sku
    );
}

/// Find routing by item_id via query param endpoint.
async fn find_routing_by_item(
    client: &reqwest::Client,
    production_url: &str,
    item_id: Uuid,
) -> Result<Uuid> {
    let url = format!("{}/api/production/routings/by-item", production_url);

    let resp = client
        .get(&url)
        .query(&[
            ("item_id", item_id.to_string()),
            ("effective_date", "2026-01-01".to_string()),
        ])
        .send()
        .await
        .with_context(|| {
            format!(
                "GET /api/production/routings/by-item?item_id={} network error",
                item_id
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/production/routings/by-item?item_id={} failed {status}: {text}",
            item_id
        );
    }

    let entry: RoutingByItemEntry = resp.json().await.with_context(|| {
        format!(
            "Failed to parse routing-by-item response for item {}",
            item_id
        )
    })?;

    Ok(entry.id)
}

/// Add a step to a routing. 409 = step already exists (idempotent).
async fn add_routing_step(
    client: &reqwest::Client,
    production_url: &str,
    routing_id: Uuid,
    sequence: i32,
    workcenter_id: Uuid,
    step: &RoutingStepDef,
) -> Result<()> {
    let url = format!(
        "{}/api/production/routings/{}/steps",
        production_url, routing_id
    );

    let body = AddRoutingStepRequest {
        sequence_number: sequence,
        workcenter_id,
        operation_name: step.operation_name.to_string(),
        description: Some(step.description.to_string()),
        setup_time_minutes: Some(step.setup_time_minutes),
        run_time_minutes: Some(step.run_time_minutes),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| {
            format!(
                "POST /api/production/routings/{}/steps (seq {}) network error",
                routing_id, sequence
            )
        })?;

    let status = resp.status();

    if status == reqwest::StatusCode::CONFLICT {
        info!(
            routing_id = %routing_id,
            sequence,
            operation = step.operation_name,
            "Routing step already exists"
        );
        return Ok(());
    }

    if status == reqwest::StatusCode::CREATED || status.is_success() {
        return Ok(());
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/production/routings/{}/steps (seq {}) failed {status}: {text}",
        routing_id,
        sequence
    );
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
    item_ids: &HashMap<String, Uuid>,
    tracker: &mut DigestTracker,
) -> Result<ProductionIds> {
    // --- Work centers (must be created first — routing steps reference them) ---
    let mut wc_map: HashMap<String, Uuid> = HashMap::new();

    for wc in WORKCENTERS {
        let wc_id = create_workcenter(client, production_url, wc).await?;
        tracker.record_workcenter(wc_id, wc.code);
        wc_map.insert(wc.code.to_string(), wc_id);
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

    for routing in ROUTINGS {
        let item_id = item_ids.get(routing.item_sku).ok_or_else(|| {
            anyhow::anyhow!(
                "Item SKU '{}' not found in inventory IDs — was inventory seeded first?",
                routing.item_sku
            )
        })?;

        let routing_id = create_routing(client, production_url, routing, *item_id).await?;
        tracker.record_routing(routing_id, *item_id, "1");

        // Add steps in manufacturing sequence order
        for (idx, step) in routing.steps.iter().enumerate() {
            let wc_id = wc_map.get(step.workcenter_code).ok_or_else(|| {
                anyhow::anyhow!(
                    "Workcenter code '{}' not found — check WORKCENTERS definition",
                    step.workcenter_code
                )
            })?;

            let sequence = (idx as i32) + 1; // 1-based
            add_routing_step(client, production_url, routing_id, sequence, *wc_id, step).await?;

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
    })
}

/// Summary of created production resources
pub struct ProductionIds {
    pub workcenters: usize,
    pub routings: usize,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_workcenters_defined() {
        assert_eq!(WORKCENTERS.len(), 6, "Expected 6 work centers");
    }

    #[test]
    fn five_routings_defined() {
        assert_eq!(ROUTINGS.len(), 5, "Expected 5 routings (one per make item)");
    }

    #[test]
    fn workcenter_codes_are_unique() {
        let mut codes: Vec<&str> = WORKCENTERS.iter().map(|w| w.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), WORKCENTERS.len(), "Duplicate workcenter codes");
    }

    #[test]
    fn all_workcenters_have_positive_capacity() {
        for wc in WORKCENTERS {
            assert!(wc.capacity > 0, "Workcenter {} has zero capacity", wc.code);
        }
    }

    #[test]
    fn all_workcenters_have_positive_cost_rate() {
        for wc in WORKCENTERS {
            assert!(
                wc.cost_rate_minor > 0,
                "Workcenter {} has zero cost rate",
                wc.code
            );
        }
    }

    #[test]
    fn routing_steps_reference_valid_workcenters() {
        let wc_codes: Vec<&str> = WORKCENTERS.iter().map(|w| w.code).collect();
        for routing in ROUTINGS {
            for step in routing.steps {
                assert!(
                    wc_codes.contains(&step.workcenter_code),
                    "Routing '{}' step '{}' references unknown workcenter '{}'",
                    routing.item_sku,
                    step.operation_name,
                    step.workcenter_code
                );
            }
        }
    }

    #[test]
    fn routing_steps_have_positive_times() {
        for routing in ROUTINGS {
            for step in routing.steps {
                assert!(
                    step.setup_time_minutes > 0,
                    "Routing '{}' step '{}' has zero setup time",
                    routing.item_sku,
                    step.operation_name
                );
                assert!(
                    step.run_time_minutes > 0,
                    "Routing '{}' step '{}' has zero run time",
                    routing.item_sku,
                    step.operation_name
                );
            }
        }
    }

    #[test]
    fn turbine_blade_has_five_steps() {
        let r = ROUTINGS.iter().find(|r| r.item_sku == "TBB-ASSY-001").unwrap();
        assert_eq!(r.steps.len(), 5, "Turbine blade should have 5 steps");
    }

    #[test]
    fn engine_mount_has_three_steps() {
        let r = ROUTINGS.iter().find(|r| r.item_sku == "EMB-ASSY-001").unwrap();
        assert_eq!(r.steps.len(), 3, "Engine mount bracket should have 3 steps");
    }

    #[test]
    fn structural_rib_has_four_steps() {
        let r = ROUTINGS.iter().find(|r| r.item_sku == "SRA-ASSY-001").unwrap();
        assert_eq!(r.steps.len(), 4, "Structural rib should have 4 steps");
    }

    #[test]
    fn fuel_line_has_three_steps() {
        let r = ROUTINGS.iter().find(|r| r.item_sku == "FLC-ASSY-001").unwrap();
        assert_eq!(r.steps.len(), 3, "Fuel line connector should have 3 steps");
    }

    #[test]
    fn landing_gear_has_five_steps() {
        let r = ROUTINGS.iter().find(|r| r.item_sku == "LGA-ASSY-001").unwrap();
        assert_eq!(r.steps.len(), 5, "Landing gear housing should have 5 steps");
    }

    #[test]
    fn heat_treat_before_grinding() {
        for routing in ROUTINGS {
            let ht_pos = routing
                .steps
                .iter()
                .position(|s| s.workcenter_code == "HEAT-TREAT");
            let grind_pos = routing
                .steps
                .iter()
                .position(|s| s.workcenter_code == "GRIND-01");
            if let (Some(ht), Some(gr)) = (ht_pos, grind_pos) {
                assert!(
                    ht < gr,
                    "Routing '{}': heat treat (pos {}) must come before grind (pos {})",
                    routing.item_sku,
                    ht,
                    gr
                );
            }
        }
    }

    #[test]
    fn ndt_before_assembly() {
        for routing in ROUTINGS {
            let ndt_pos = routing
                .steps
                .iter()
                .position(|s| s.workcenter_code == "NDT-01");
            let assy_pos = routing
                .steps
                .iter()
                .position(|s| s.workcenter_code == "ASSEMBLY-01");
            if let (Some(ndt), Some(assy)) = (ndt_pos, assy_pos) {
                assert!(
                    ndt < assy,
                    "Routing '{}': NDT (pos {}) must come before assembly (pos {})",
                    routing.item_sku,
                    ndt,
                    assy
                );
            }
        }
    }

    #[test]
    fn rough_before_finish_on_turbine_blade() {
        let r = ROUTINGS.iter().find(|r| r.item_sku == "TBB-ASSY-001").unwrap();
        let rough_pos = r
            .steps
            .iter()
            .position(|s| s.operation_name == "Rough Mill")
            .expect("Turbine blade should have Rough Mill");
        let finish_pos = r
            .steps
            .iter()
            .position(|s| s.operation_name == "Finish Mill")
            .expect("Turbine blade should have Finish Mill");
        assert!(
            rough_pos < finish_pos,
            "Rough mill (pos {}) must come before finish mill (pos {})",
            rough_pos,
            finish_pos
        );
    }

    #[test]
    fn all_routing_skus_are_make_items() {
        let make_skus = [
            "TBB-ASSY-001",
            "EMB-ASSY-001",
            "SRA-ASSY-001",
            "FLC-ASSY-001",
            "LGA-ASSY-001",
        ];
        for routing in ROUTINGS {
            assert!(
                make_skus.contains(&routing.item_sku),
                "Routing references non-make SKU: {}",
                routing.item_sku
            );
        }
    }

    #[test]
    fn digest_records_workcenters() {
        let mut tracker = DigestTracker::new();
        let id = Uuid::new_v4();
        tracker.record_workcenter(id, "CNC-MILL-01");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }

    #[test]
    fn digest_records_routings() {
        let mut tracker = DigestTracker::new();
        let routing_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        tracker.record_routing(routing_id, item_id, "1");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }
}
