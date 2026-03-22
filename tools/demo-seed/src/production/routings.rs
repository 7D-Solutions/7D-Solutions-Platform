//! Routing and routing step HTTP operations for demo-seed

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

pub(super) use super::data::{RoutingDef, RoutingStepDef, ROUTINGS};

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

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
// HTTP operations
// ---------------------------------------------------------------------------

/// Create a routing; on 409, find by item_id.
pub(super) async fn create_routing(
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
pub(super) async fn add_routing_step(
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::workcenters::WORKCENTERS;

    #[test]
    fn five_routings_defined() {
        assert_eq!(ROUTINGS.len(), 5, "Expected 5 routings (one per make item)");
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
    fn digest_records_routings() {
        let mut tracker = crate::digest::DigestTracker::new();
        let routing_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        tracker.record_routing(routing_id, item_id, "1");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }
}
