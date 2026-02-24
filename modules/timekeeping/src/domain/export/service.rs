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
use crate::events;

const EVT_EXPORT_COMPLETED: &str = "export_run.completed";

const EXPORT_COLS: &str = r#"
    id, app_id, export_type, period_start, period_end, status,
    record_count, metadata, content_hash, error_message,
    started_at, completed_at, created_at
"#;

// -- Reads ------------------------------------------------------------------

pub async fn get_export_run(
    pool: &PgPool,
    app_id: &str,
    run_id: Uuid,
) -> Result<ExportRun, ExportError> {
    let sql = format!(
        "SELECT {} FROM tk_export_runs WHERE app_id = $1 AND id = $2",
        EXPORT_COLS
    );
    sqlx::query_as::<_, ExportRun>(&sql)
        .bind(app_id)
        .bind(run_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ExportError::NotFound)
}

pub async fn list_export_runs(
    pool: &PgPool,
    app_id: &str,
    export_type: Option<&str>,
) -> Result<Vec<ExportRun>, ExportError> {
    if let Some(et) = export_type {
        let sql = format!(
            "SELECT {} FROM tk_export_runs \
             WHERE app_id = $1 AND export_type = $2 \
             ORDER BY created_at DESC",
            EXPORT_COLS
        );
        Ok(sqlx::query_as::<_, ExportRun>(&sql)
            .bind(app_id)
            .bind(et)
            .fetch_all(pool)
            .await?)
    } else {
        let sql = format!(
            "SELECT {} FROM tk_export_runs WHERE app_id = $1 ORDER BY created_at DESC",
            EXPORT_COLS
        );
        Ok(sqlx::query_as::<_, ExportRun>(&sql)
            .bind(app_id)
            .fetch_all(pool)
            .await?)
    }
}

// -- Create export run -------------------------------------------------------

pub async fn create_export_run(
    pool: &PgPool,
    req: &CreateExportRunRequest,
) -> Result<ExportArtifact, ExportError> {
    validate_request(req)?;

    let now = Utc::now();

    // Fetch approved entries for the period (deterministic order)
    let entries = fetch_approved_entries(
        pool,
        &req.app_id,
        req.period_start,
        req.period_end,
    )
    .await?;

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
    let json_canonical = serde_json::to_string(&json_content)
        .unwrap_or_default();
    let content_hash = compute_hash(&csv_content, &json_canonical);

    // Check for idempotent replay — same app + type + period + hash
    if let Some(existing) = find_by_hash(
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

    let run = sqlx::query_as::<_, ExportRun>(&format!(
        r#"INSERT INTO tk_export_runs
            (id, app_id, export_type, period_start, period_end, status,
             record_count, metadata, content_hash, started_at, completed_at)
        VALUES ($1, $2, $3, $4, $5, 'completed', $6, $7, $8, $9, $9)
        RETURNING {}"#,
        EXPORT_COLS
    ))
    .bind(run_id)
    .bind(&req.app_id)
    .bind(&req.export_type)
    .bind(req.period_start)
    .bind(req.period_end)
    .bind(record_count)
    .bind(&json_content)
    .bind(&content_hash)
    .bind(now)
    .fetch_one(&mut *tx)
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

/// Fetch current (non-void) entries from approved periods, joined with
/// employee and project names for the export. Sorted deterministically
/// by (work_date, employee_id, entry_id).
async fn fetch_approved_entries(
    pool: &PgPool,
    app_id: &str,
    period_start: chrono::NaiveDate,
    period_end: chrono::NaiveDate,
) -> Result<Vec<ExportEntry>, ExportError> {
    let rows = sqlx::query_as::<_, ExportEntry>(
        r#"
        SELECT
            e.entry_id,
            e.employee_id,
            COALESCE(emp.first_name || ' ' || emp.last_name, 'Unknown') AS employee_name,
            e.project_id,
            p.name AS project_name,
            e.task_id,
            e.work_date,
            e.minutes,
            e.description
        FROM tk_timesheet_entries e
        JOIN tk_approval_requests ar
            ON ar.app_id = e.app_id
            AND ar.employee_id = e.employee_id
            AND ar.period_start <= e.work_date
            AND ar.period_end >= e.work_date
            AND ar.status = 'approved'
        LEFT JOIN tk_employees emp ON emp.id = e.employee_id
        LEFT JOIN tk_projects p ON p.id = e.project_id
        WHERE e.app_id = $1
          AND e.work_date >= $2
          AND e.work_date <= $3
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
        ORDER BY e.work_date, e.employee_id, e.entry_id
        "#,
    )
    .bind(app_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

fn compute_hash(csv: &str, json_canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(csv.as_bytes());
    hasher.update(b"|");
    hasher.update(json_canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

async fn find_by_hash(
    pool: &PgPool,
    app_id: &str,
    export_type: &str,
    period_start: chrono::NaiveDate,
    period_end: chrono::NaiveDate,
    content_hash: &str,
) -> Result<Option<ExportRun>, ExportError> {
    let sql = format!(
        "SELECT {} FROM tk_export_runs \
         WHERE app_id = $1 AND export_type = $2 \
         AND period_start = $3 AND period_end = $4 \
         AND content_hash = $5 LIMIT 1",
        EXPORT_COLS
    );
    Ok(sqlx::query_as::<_, ExportRun>(&sql)
        .bind(app_id)
        .bind(export_type)
        .bind(period_start)
        .bind(period_end)
        .bind(content_hash)
        .fetch_optional(pool)
        .await?)
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
