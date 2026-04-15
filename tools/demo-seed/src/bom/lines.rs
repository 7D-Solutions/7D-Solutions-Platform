//! BOM line (component) HTTP operations

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

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
pub(super) struct LineResponse {
    pub id: Uuid,
    pub component_item_id: Uuid,
}

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

/// Get existing lines for a revision.
pub(super) async fn get_revision_lines(
    client: &reqwest::Client,
    bom_url: &str,
    revision_id: Uuid,
) -> Result<Vec<LineResponse>> {
    let url = format!("{}/api/bom/revisions/{}/lines", bom_url, revision_id);
    let resp =
        client.get(&url).send().await.with_context(|| {
            format!("GET /api/bom/revisions/{}/lines network error", revision_id)
        })?;

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
pub(super) async fn create_line(
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
