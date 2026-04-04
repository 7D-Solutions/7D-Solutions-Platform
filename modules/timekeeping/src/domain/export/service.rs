//! Export run service — Guard→Mutation→Outbox atomicity.
//!
//! Creates export runs from approved timesheet entries.
//! Generates deterministic CSV + JSON artifacts and stores a content hash.
//! Re-running the same export with unchanged data yields the same hash.

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::{csv, json, models::*};
use super::repo;
use crate::events;

const EVT_EXPORT_COMPLETED: &str = "export_run.completed";

// -- Reads ------------------------------------------------------------------

pub async fn get_export_run(
    pool: &PgPool,
    app_id: &str,
    run_id: Uuid,
) -> Result<ExportRun, ExportError> {
    repo::fetch_export_run(pool, app_id, run_id)
        .await?
        .ok_or(ExportError::NotFound)
}

pub async fn list_export_runs(
    pool: &PgPool,
    app_id: &str,
    export_type: Option<&str>,
) -> Result<Vec<ExportRun>, ExportError> {
    repo::list_export_runs(pool, app_id, export_type).await
}

// -- Create export run -------------------------------------------------------

pub async fn create_export_run(
    pool: &PgPool,
    req: &CreateExportRunRequest,
) -> Result<ExportArtifact, ExportError> {
    validate_request(req)?;

    let now = Utc::now();

    // Fetch approved entries for the period (deterministic order)
    let entries =
        repo::fetch_approved_entries(pool, &req.app_id, req.period_start, req.period_end).await?;

    if entries.is_empty() {
        return Err(ExportError::NoApprovedEntries);
    }

    // Generate deterministic artifacts
    let csv_content = csv::generate(&entries);
    let json_content = json::generate(
        &req.app_id,
        &req.export_type,
        req.period_start,
        req.period_end,
        &entries,
    );

    // Compute content hash (SHA-256 of CSV + canonical JSON)
    let json_canonical = serde_json::to_string(&json_content).unwrap_or_default();
    let content_hash = compute_hash(&csv_content, &json_canonical);

    // Check for idempotent replay — same app + type + period + hash
    if let Some(existing) = repo::find_by_hash(
        pool,
        &req.app_id,
        &req.export_type,
        req.period_start,
        req.period_end,
        &content_hash,
    )
    .await?
    {
        return Err(ExportError::IdempotentReplay {
            run_id: existing.id,
        });
    }

    // Create the export run atomically
    let run_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let record_count = entries.len() as i32;

    let mut tx = pool.begin().await?;

    let run = repo::insert_export_run(
        &mut *tx,
        run_id,
        &req.app_id,
        &req.export_type,
        req.period_start,
        req.period_end,
        record_count,
        &json_content,
        &content_hash,
        now,
    )
    .await?;

    let payload = serde_json::json!({
        "run_id": run_id,
        "app_id": req.app_id,
        "export_type": req.export_type,
        "period_start": req.period_start,
        "period_end": req.period_end,
        "record_count": record_count,
        "content_hash": content_hash,
    });
    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_EXPORT_COMPLETED,
        "export_run",
        &run_id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    Ok(ExportArtifact {
        run,
        csv: csv_content,
        json: json_content,
    })
}

// -- Internal helpers --------------------------------------------------------

fn compute_hash(csv: &str, json_canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(csv.as_bytes());
    hasher.update(b"|");
    hasher.update(json_canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn validate_request(req: &CreateExportRunRequest) -> Result<(), ExportError> {
    if req.app_id.trim().is_empty() {
        return Err(ExportError::Validation("app_id must not be empty".into()));
    }
    if req.export_type.trim().is_empty() {
        return Err(ExportError::Validation(
            "export_type must not be empty".into(),
        ));
    }
    if req.period_end < req.period_start {
        return Err(ExportError::Validation(
            "period_end must be >= period_start".into(),
        ));
    }
    Ok(())
}
