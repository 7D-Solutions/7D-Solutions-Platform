//! Work center seeding for demo-seed

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

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

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

pub(super) struct WorkcenterDef {
    pub code: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub capacity: i32,
    pub cost_rate_minor: i64,
}

pub(super) const WORKCENTERS: &[WorkcenterDef] = &[
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

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

/// Create a work center; on 409, list all and find by code.
pub(super) async fn create_workcenter(
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::DigestTracker;

    #[test]
    fn six_workcenters_defined() {
        assert_eq!(WORKCENTERS.len(), 6, "Expected 6 work centers");
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
    fn digest_records_workcenters() {
        let mut tracker = DigestTracker::new();
        let id = Uuid::new_v4();
        tracker.record_workcenter(id, "CNC-MILL-01");
        let digest = tracker.finalize();
        assert_eq!(digest.len(), 64, "SHA256 hex should be 64 chars");
    }
}
