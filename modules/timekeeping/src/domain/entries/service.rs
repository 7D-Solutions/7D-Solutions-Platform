//! Timesheet entry service — Guard→Mutation→Outbox atomicity.
//!
//! Append-only: entries are never updated. Corrections insert a new row
//! with the same entry_id, incremented version, and the previous version's
//! is_current flipped to FALSE. Voiding sets minutes=0 with entry_type='void'.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::guards;
use super::models::*;
use super::repo;
use crate::events;

const EVT_ENTRY_CREATED: &str = "timesheet_entry.created";
const EVT_ENTRY_CORRECTED: &str = "timesheet_entry.corrected";
const EVT_ENTRY_VOIDED: &str = "timesheet_entry.voided";

// ============================================================================
// Reads
// ============================================================================

/// Fetch current entries for an employee within a date range.
pub async fn list_entries(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<TimesheetEntry>, EntryError> {
    repo::list_entries(pool, app_id, employee_id, from, to).await
}

/// Fetch the full version history for a logical entry.
pub async fn entry_history(
    pool: &PgPool,
    app_id: &str,
    entry_id: Uuid,
) -> Result<Vec<TimesheetEntry>, EntryError> {
    let rows = repo::entry_history(pool, app_id, entry_id).await?;

    if rows.is_empty() {
        return Err(EntryError::NotFound);
    }

    Ok(rows)
}

// ============================================================================
// Create (original entry)
// ============================================================================

pub async fn create_entry(
    pool: &PgPool,
    req: &CreateEntryRequest,
    idempotency_key: Option<&str>,
) -> Result<TimesheetEntry, EntryError> {
    // Guard
    guards::validate_create(req)?;

    if let Some(key) = idempotency_key {
        if let Some((body, code)) = events::check_idempotency(pool, &req.app_id, key).await? {
            return Err(EntryError::IdempotentReplay {
                status_code: code as u16,
                body,
            });
        }
    }

    guards::check_period_lock(pool, &req.app_id, req.employee_id, req.work_date).await?;
    guards::check_overlap(
        pool,
        &req.app_id,
        req.employee_id,
        req.work_date,
        req.project_id,
        req.task_id,
    )
    .await?;

    // Mutation + Outbox (atomic)
    let entry_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    let entry = repo::insert_entry(
        &mut *tx,
        entry_id,
        &req.app_id,
        req.employee_id,
        req.project_id,
        req.task_id,
        req.work_date,
        req.minutes,
        req.description.as_deref(),
        req.created_by,
    )
    .await?;

    let payload = serde_json::json!({
        "entry_id": entry_id,
        "app_id": req.app_id,
        "employee_id": req.employee_id,
        "work_date": req.work_date,
        "minutes": req.minutes,
        "version": 1,
    });

    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ENTRY_CREATED,
        "timesheet_entry",
        &entry_id.to_string(),
        &payload,
    )
    .await?;

    if let Some(key) = idempotency_key {
        events::record_idempotency(&mut tx, &req.app_id, key, &entry, 201).await?;
    }

    tx.commit().await?;

    Ok(entry)
}

// ============================================================================
// Correct (compensating entry)
// ============================================================================

pub async fn correct_entry(
    pool: &PgPool,
    req: &CorrectEntryRequest,
    idempotency_key: Option<&str>,
) -> Result<TimesheetEntry, EntryError> {
    // Guard
    guards::validate_correct(req)?;

    if let Some(key) = idempotency_key {
        if let Some((body, code)) = events::check_idempotency(pool, &req.app_id, key).await? {
            return Err(EntryError::IdempotentReplay {
                status_code: code as u16,
                body,
            });
        }
    }

    let mut tx = pool.begin().await?;

    // Fetch current version (FOR UPDATE to prevent concurrent corrections)
    let current = repo::fetch_current_for_update(&mut *tx, &req.app_id, req.entry_id)
        .await?
        .ok_or(EntryError::NotFound)?;

    // Check period lock using the original work_date
    guards::check_period_lock(pool, &req.app_id, current.employee_id, current.work_date).await?;

    let new_version = current.version + 1;
    let event_id = Uuid::new_v4();

    // Flip old version's is_current to FALSE
    repo::flip_is_current(&mut *tx, req.entry_id).await?;

    // Use corrected project/task or inherit from current
    let project_id = req.project_id.or(current.project_id);
    let task_id = req.task_id.or(current.task_id);

    // Insert correction row
    let entry = repo::insert_correction(
        &mut *tx,
        req.entry_id,
        new_version,
        &req.app_id,
        current.employee_id,
        project_id,
        task_id,
        current.work_date,
        req.minutes,
        req.description.as_deref(),
        req.created_by,
    )
    .await?;

    let payload = serde_json::json!({
        "entry_id": req.entry_id,
        "app_id": req.app_id,
        "employee_id": current.employee_id,
        "work_date": current.work_date,
        "old_minutes": current.minutes,
        "new_minutes": req.minutes,
        "version": new_version,
    });

    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ENTRY_CORRECTED,
        "timesheet_entry",
        &req.entry_id.to_string(),
        &payload,
    )
    .await?;

    if let Some(key) = idempotency_key {
        events::record_idempotency(&mut tx, &req.app_id, key, &entry, 200).await?;
    }

    tx.commit().await?;

    Ok(entry)
}

// ============================================================================
// Void (cancel entry with minutes=0)
// ============================================================================

pub async fn void_entry(
    pool: &PgPool,
    req: &VoidEntryRequest,
    idempotency_key: Option<&str>,
) -> Result<TimesheetEntry, EntryError> {
    // Guard
    guards::validate_void(req)?;

    if let Some(key) = idempotency_key {
        if let Some((body, code)) = events::check_idempotency(pool, &req.app_id, key).await? {
            return Err(EntryError::IdempotentReplay {
                status_code: code as u16,
                body,
            });
        }
    }

    let mut tx = pool.begin().await?;

    let current = repo::fetch_current_for_update(&mut *tx, &req.app_id, req.entry_id)
        .await?
        .ok_or(EntryError::NotFound)?;

    guards::check_period_lock(pool, &req.app_id, current.employee_id, current.work_date).await?;

    let new_version = current.version + 1;
    let event_id = Uuid::new_v4();

    // Flip old version
    repo::flip_is_current(&mut *tx, req.entry_id).await?;

    // Insert void row (minutes=0)
    let entry = repo::insert_void(
        &mut *tx,
        req.entry_id,
        new_version,
        &req.app_id,
        current.employee_id,
        current.project_id,
        current.task_id,
        current.work_date,
        req.created_by,
    )
    .await?;

    let payload = serde_json::json!({
        "entry_id": req.entry_id,
        "app_id": req.app_id,
        "employee_id": current.employee_id,
        "work_date": current.work_date,
        "voided_minutes": current.minutes,
        "version": new_version,
    });

    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_ENTRY_VOIDED,
        "timesheet_entry",
        &req.entry_id.to_string(),
        &payload,
    )
    .await?;

    if let Some(key) = idempotency_key {
        events::record_idempotency(&mut tx, &req.app_id, key, &entry, 200).await?;
    }

    tx.commit().await?;

    Ok(entry)
}
