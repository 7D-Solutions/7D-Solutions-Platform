//! BOM header, revision, and effectivity HTTP operations

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixed effectivity date (deterministic across runs)
// ---------------------------------------------------------------------------

pub(super) fn effectivity_from() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("Fixed effectivity date must parse")
}

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateBomRequest {
    part_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BomHeaderResponse {
    pub id: Uuid,
}

#[derive(Serialize)]
struct CreateRevisionRequest {
    revision_label: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RevisionResponse {
    pub id: Uuid,
    pub revision_label: String,
}

#[derive(Serialize)]
struct SetEffectivityRequest {
    effective_from: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_to: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// HTTP operations
// ---------------------------------------------------------------------------

/// Check if a BOM already exists for this part, or create one.
pub(super) async fn get_or_create_bom(
    client: &reqwest::Client,
    bom_url: &str,
    part_id: Uuid,
    description: &str,
) -> Result<Uuid> {
    // GET /api/bom/by-part/{part_id}
    let get_url = format!("{}/api/bom/by-part/{}", bom_url, part_id);
    let resp = client
        .get(&get_url)
        .send()
        .await
        .with_context(|| format!("GET /api/bom/by-part/{} network error", part_id))?;

    if resp.status().is_success() {
        let bom: BomHeaderResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse BOM by-part response for {}", part_id))?;
        info!(bom_id = %bom.id, part_id = %part_id, "BOM already exists");
        return Ok(bom.id);
    }

    if resp.status() != reqwest::StatusCode::NOT_FOUND {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/bom/by-part/{} unexpected {}: {}",
            part_id,
            status,
            text
        );
    }

    // 404 — create new BOM
    let post_url = format!("{}/api/bom", bom_url);
    let body = CreateBomRequest {
        part_id,
        description: Some(description.to_string()),
    };

    let resp = client
        .post(&post_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/bom for part {} network error", part_id))?;

    let status = resp.status();
    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let bom: BomHeaderResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse BOM creation response")?;
        return Ok(bom.id);
    }

    if status == reqwest::StatusCode::CONFLICT {
        // Concurrent runner created the BOM between our GET and POST — re-fetch
        info!(part_id = %part_id, "BOM created concurrently — re-fetching by part");
        let resp = client
            .get(&get_url)
            .send()
            .await
            .with_context(|| format!("GET /api/bom/by-part/{} (retry) network error", part_id))?;
        let bom: BomHeaderResponse = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse BOM by-part retry response for {}", part_id))?;
        return Ok(bom.id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom for part {} failed {}: {}",
        part_id,
        status,
        text
    );
}

/// Check if revision A exists, or create it.
pub(super) async fn get_or_create_revision(
    client: &reqwest::Client,
    bom_url: &str,
    bom_id: Uuid,
    label: &str,
) -> Result<Uuid> {
    // GET /api/bom/{bom_id}/revisions
    let list_url = format!("{}/api/bom/{}/revisions", bom_url, bom_id);
    let resp = client
        .get(&list_url)
        .send()
        .await
        .with_context(|| format!("GET /api/bom/{}/revisions network error", bom_id))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "GET /api/bom/{}/revisions failed {}: {}",
            bom_id,
            status,
            text
        );
    }

    let revisions: Vec<RevisionResponse> = resp
        .json()
        .await
        .with_context(|| format!("Failed to parse revisions for BOM {}", bom_id))?;

    if let Some(rev) = revisions.iter().find(|r| r.revision_label == label) {
        info!(revision_id = %rev.id, label, "Revision already exists");
        return Ok(rev.id);
    }

    // Create revision
    let post_url = format!("{}/api/bom/{}/revisions", bom_url, bom_id);
    let body = CreateRevisionRequest {
        revision_label: label.to_string(),
    };

    let resp = client
        .post(&post_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/bom/{}/revisions network error", bom_id))?;

    let status = resp.status();
    if status == reqwest::StatusCode::CREATED || status.is_success() {
        let rev: RevisionResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse revision creation response")?;
        return Ok(rev.id);
    }

    if status == reqwest::StatusCode::CONFLICT {
        // Concurrent runner created the revision — re-fetch the list
        info!(bom_id = %bom_id, label, "Revision created concurrently — re-fetching revisions");
        let resp = client
            .get(&list_url)
            .send()
            .await
            .with_context(|| format!("GET /api/bom/{}/revisions (retry) network error", bom_id))?;
        let revisions: Vec<RevisionResponse> = resp
            .json()
            .await
            .with_context(|| format!("Failed to parse revisions retry for BOM {}", bom_id))?;
        if let Some(rev) = revisions.iter().find(|r| r.revision_label == label) {
            return Ok(rev.id);
        }
        bail!("Revision '{}' not found in BOM {} after concurrent conflict", label, bom_id);
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom/{}/revisions failed {}: {}",
        bom_id,
        status,
        text
    );
}

/// Set effectivity dates on a revision. Treats 409 as already-set success.
pub(super) async fn set_effectivity(
    client: &reqwest::Client,
    bom_url: &str,
    revision_id: Uuid,
    effective_from: DateTime<Utc>,
) -> Result<()> {
    let url = format!("{}/api/bom/revisions/{}/effectivity", bom_url, revision_id);
    let body = SetEffectivityRequest {
        effective_from,
        effective_to: None,
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| {
            format!(
                "POST /api/bom/revisions/{}/effectivity network error",
                revision_id
            )
        })?;

    let status = resp.status();
    if status.is_success() || status == reqwest::StatusCode::CONFLICT {
        if status == reqwest::StatusCode::CONFLICT {
            info!(revision_id = %revision_id, "Effectivity already set");
        }
        return Ok(());
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/bom/revisions/{}/effectivity failed {}: {}",
        revision_id,
        status,
        text
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effectivity_from_is_deterministic() {
        let d1 = effectivity_from();
        let d2 = effectivity_from();
        assert_eq!(d1, d2);
        assert_eq!(d1.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    }
}
