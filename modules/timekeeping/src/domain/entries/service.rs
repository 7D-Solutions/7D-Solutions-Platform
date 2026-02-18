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
    let rows = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND employee_id = $2
          AND work_date >= $3 AND work_date <= $4
          AND is_current = TRUE
        ORDER BY work_date, created_at
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Fetch the full version history for a logical entry.
pub async fn entry_history(
    pool: &PgPool,
    app_id: &str,
    entry_id: Uuid,
) -> Result<Vec<TimesheetEntry>, EntryError> {
    let rows = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND entry_id = $2
        ORDER BY version ASC
        "#,
    )
    .bind(app_id)
    .bind(entry_id)
    .fetch_all(pool)
    .await?;

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

    let entry = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id, task_id,
             work_date, minutes, description, entry_type, is_current, created_by)
        VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, 'original', TRUE, $9)
        RETURNING id, entry_id, version, app_id, employee_id, project_id, task_id,
                  work_date, minutes, description, entry_type, is_current,
                  created_by, created_at
        "#,
    )
    .bind(entry_id)
    .bind(&req.app_id)
    .bind(req.employee_id)
    .bind(req.project_id)
    .bind(req.task_id)
    .bind(req.work_date)
    .bind(req.minutes)
    .bind(req.description.as_deref())
    .bind(req.created_by)
    .fetch_one(&mut *tx)
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
    let current = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND entry_id = $2 AND is_current = TRUE
        FOR UPDATE
        "#,
    )
    .bind(&req.app_id)
    .bind(req.entry_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(EntryError::NotFound)?;

    // Check period lock using the original work_date
    guards::check_period_lock(pool, &req.app_id, current.employee_id, current.work_date).await?;

    let new_version = current.version + 1;
    let event_id = Uuid::new_v4();

    // Flip old version's is_current to FALSE
    sqlx::query(
        "UPDATE tk_timesheet_entries SET is_current = FALSE \
         WHERE entry_id = $1 AND is_current = TRUE",
    )
    .bind(req.entry_id)
    .execute(&mut *tx)
    .await?;

    // Use corrected project/task or inherit from current
    let project_id = req.project_id.or(current.project_id);
    let task_id = req.task_id.or(current.task_id);

    // Insert correction row
    let entry = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id, task_id,
             work_date, minutes, description, entry_type, is_current, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'correction', TRUE, $10)
        RETURNING id, entry_id, version, app_id, employee_id, project_id, task_id,
                  work_date, minutes, description, entry_type, is_current,
                  created_by, created_at
        "#,
    )
    .bind(req.entry_id)
    .bind(new_version)
    .bind(&req.app_id)
    .bind(current.employee_id)
    .bind(project_id)
    .bind(task_id)
    .bind(current.work_date)
    .bind(req.minutes)
    .bind(req.description.as_deref())
    .bind(req.created_by)
    .fetch_one(&mut *tx)
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

    let current = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND entry_id = $2 AND is_current = TRUE
        FOR UPDATE
        "#,
    )
    .bind(&req.app_id)
    .bind(req.entry_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(EntryError::NotFound)?;

    guards::check_period_lock(pool, &req.app_id, current.employee_id, current.work_date).await?;

    let new_version = current.version + 1;
    let event_id = Uuid::new_v4();

    // Flip old version
    sqlx::query(
        "UPDATE tk_timesheet_entries SET is_current = FALSE \
         WHERE entry_id = $1 AND is_current = TRUE",
    )
    .bind(req.entry_id)
    .execute(&mut *tx)
    .await?;

    // Insert void row (minutes=0)
    let entry = sqlx::query_as::<_, TimesheetEntry>(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id, task_id,
             work_date, minutes, description, entry_type, is_current, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'Voided', 'void', TRUE, $8)
        RETURNING id, entry_id, version, app_id, employee_id, project_id, task_id,
                  work_date, minutes, description, entry_type, is_current,
                  created_by, created_at
        "#,
    )
    .bind(req.entry_id)
    .bind(new_version)
    .bind(&req.app_id)
    .bind(current.employee_id)
    .bind(current.project_id)
    .bind(current.task_id)
    .bind(current.work_date)
    .bind(req.created_by)
    .fetch_one(&mut *tx)
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
