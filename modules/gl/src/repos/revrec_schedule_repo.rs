//! Repository for revrec schedule persistence and recognition runs
//!
//! Schedule creation persists schedule + lines + outbox event in a single transaction.
//! Recognition queries find due lines and mark them as recognized.

use sqlx::PgPool;
use uuid::Uuid;

use crate::repos::outbox_repo;
use crate::revrec::{
    ScheduleCreatedPayload, EVENT_TYPE_SCHEDULE_CREATED, MUTATION_CLASS_DATA_MUTATION,
};

use super::revrec_repo::RevrecRepoError;

/// Create a recognition schedule with lines, atomically with outbox event.
///
/// This function:
/// 1. Validates schedule lines sum == total_to_recognize_minor
/// 2. Determines version (max existing version + 1 for this obligation)
/// 3. Inserts schedule row with version and optional previous_schedule_id
/// 4. Inserts all schedule line rows
/// 5. Inserts revrec.schedule_created into outbox
/// 6. Commits atomically
///
/// Idempotency: if schedule_id already exists, returns DuplicateSchedule error.
pub async fn create_schedule(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ScheduleCreatedPayload,
) -> Result<Uuid, RevrecRepoError> {
    // Validate lines sum invariant
    let lines_sum: i64 = payload
        .lines
        .iter()
        .map(|l| l.amount_to_recognize_minor)
        .sum();
    if lines_sum != payload.total_to_recognize_minor {
        return Err(RevrecRepoError::ScheduleSumMismatch {
            sum: lines_sum,
            expected: payload.total_to_recognize_minor,
        });
    }

    let mut tx = pool.begin().await?;

    // Idempotency check
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_schedules WHERE schedule_id = $1 AND tenant_id = $2)",
    )
    .bind(payload.schedule_id)
    .bind(&payload.tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    if exists {
        tx.rollback().await?;
        return Err(RevrecRepoError::DuplicateSchedule(payload.schedule_id));
    }

    // Determine version: max existing + 1
    let current_max: Option<i32> =
        sqlx::query_scalar("SELECT MAX(version) FROM revrec_schedules WHERE obligation_id = $1")
            .bind(payload.obligation_id)
            .fetch_one(&mut *tx)
            .await?;

    let version = current_max.unwrap_or(0) + 1;

    // Find previous schedule_id (the one with the current max version)
    let previous_schedule_id: Option<Uuid> = if version > 1 {
        sqlx::query_scalar(
            "SELECT schedule_id FROM revrec_schedules
             WHERE obligation_id = $1 AND version = $2",
        )
        .bind(payload.obligation_id)
        .bind(version - 1)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };

    // Insert schedule
    sqlx::query(
        r#"
        INSERT INTO revrec_schedules (
            schedule_id, contract_id, obligation_id, tenant_id,
            total_to_recognize_minor, currency,
            first_period, last_period,
            version, previous_schedule_id,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(payload.schedule_id)
    .bind(payload.contract_id)
    .bind(payload.obligation_id)
    .bind(&payload.tenant_id)
    .bind(payload.total_to_recognize_minor)
    .bind(&payload.currency)
    .bind(&payload.first_period)
    .bind(&payload.last_period)
    .bind(version)
    .bind(previous_schedule_id)
    .bind(payload.created_at)
    .execute(&mut *tx)
    .await?;

    // Insert schedule lines
    for line in &payload.lines {
        sqlx::query(
            r#"
            INSERT INTO revrec_schedule_lines (
                schedule_id, period, amount_to_recognize_minor,
                deferred_revenue_account, recognized_revenue_account
            )
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(payload.schedule_id)
        .bind(&line.period)
        .bind(line.amount_to_recognize_minor)
        .bind(&line.deferred_revenue_account)
        .bind(&line.recognized_revenue_account)
        .execute(&mut *tx)
        .await?;
    }

    // Insert outbox event atomically
    let outbox_payload =
        serde_json::to_value(payload).map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_SCHEDULE_CREATED,
        "revrec_schedule",
        &payload.schedule_id.to_string(),
        outbox_payload,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        schedule_id = %payload.schedule_id,
        obligation_id = %payload.obligation_id,
        version = version,
        lines = payload.lines.len(),
        total = payload.total_to_recognize_minor,
        "Revrec schedule created atomically with outbox (v{})",
        version
    );

    Ok(payload.schedule_id)
}

/// Fetch a schedule by ID
pub async fn get_schedule(
    pool: &PgPool,
    schedule_id: Uuid,
) -> Result<Option<ScheduleRow>, RevrecRepoError> {
    let row = sqlx::query_as::<_, ScheduleRow>(
        "SELECT schedule_id, contract_id, obligation_id, tenant_id,
                total_to_recognize_minor, currency, first_period, last_period,
                version, previous_schedule_id, created_at
         FROM revrec_schedules
         WHERE schedule_id = $1",
    )
    .bind(schedule_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Fetch schedule lines for a schedule
pub async fn get_schedule_lines(
    pool: &PgPool,
    schedule_id: Uuid,
) -> Result<Vec<ScheduleLineRow>, RevrecRepoError> {
    let rows = sqlx::query_as::<_, ScheduleLineRow>(
        "SELECT id, schedule_id, period, amount_to_recognize_minor,
                deferred_revenue_account, recognized_revenue_account,
                recognized, recognized_at
         FROM revrec_schedule_lines
         WHERE schedule_id = $1
         ORDER BY period",
    )
    .bind(schedule_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Get the latest schedule version for an obligation
pub async fn get_latest_schedule_for_obligation(
    pool: &PgPool,
    obligation_id: Uuid,
) -> Result<Option<ScheduleRow>, RevrecRepoError> {
    let row = sqlx::query_as::<_, ScheduleRow>(
        "SELECT schedule_id, contract_id, obligation_id, tenant_id,
                total_to_recognize_minor, currency, first_period, last_period,
                version, previous_schedule_id, created_at
         FROM revrec_schedules
         WHERE obligation_id = $1
         ORDER BY version DESC
         LIMIT 1",
    )
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleRow {
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub tenant_id: String,
    pub total_to_recognize_minor: i64,
    pub currency: String,
    pub first_period: String,
    pub last_period: String,
    pub version: i32,
    pub previous_schedule_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleLineRow {
    pub id: i64,
    pub schedule_id: Uuid,
    pub period: String,
    pub amount_to_recognize_minor: i64,
    pub deferred_revenue_account: String,
    pub recognized_revenue_account: String,
    pub recognized: bool,
    pub recognized_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ============================================================================
// Recognition Run Support
// ============================================================================

/// A due schedule line enriched with contract/obligation context for recognition posting.
#[derive(Debug, sqlx::FromRow)]
pub struct DueScheduleLine {
    pub line_id: i64,
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub tenant_id: String,
    pub period: String,
    pub amount_to_recognize_minor: i64,
    pub currency: String,
    pub deferred_revenue_account: String,
    pub recognized_revenue_account: String,
}

/// Find unrecognized schedule lines due for a given period.
///
/// Only returns lines from the **latest** schedule version for each obligation,
/// preventing double-recognition when schedules are re-versioned.
///
/// The query:
/// 1. Finds the max version per obligation_id
/// 2. Joins to schedule_lines WHERE recognized = false AND period = target
/// 3. Returns enriched rows with contract/obligation context
pub async fn find_due_lines_for_period(
    pool: &PgPool,
    tenant_id: &str,
    period: &str,
) -> Result<Vec<DueScheduleLine>, RevrecRepoError> {
    let rows = sqlx::query_as::<_, DueScheduleLine>(
        r#"
        SELECT
            sl.id AS line_id,
            s.schedule_id,
            s.contract_id,
            s.obligation_id,
            s.tenant_id,
            sl.period,
            sl.amount_to_recognize_minor,
            s.currency,
            sl.deferred_revenue_account,
            sl.recognized_revenue_account
        FROM revrec_schedule_lines sl
        JOIN revrec_schedules s ON sl.schedule_id = s.schedule_id
        JOIN (
            SELECT obligation_id, MAX(version) AS max_version
            FROM revrec_schedules
            WHERE tenant_id = $1
            GROUP BY obligation_id
        ) latest ON s.obligation_id = latest.obligation_id AND s.version = latest.max_version
        WHERE s.tenant_id = $1
          AND sl.period = $2
          AND sl.recognized = false
        ORDER BY sl.id
        "#,
    )
    .bind(tenant_id)
    .bind(period)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a schedule line as recognized within an existing transaction.
///
/// Sets `recognized = true` and `recognized_at = NOW()`.
/// Returns the number of rows affected (0 if already recognized — idempotent).
pub async fn mark_line_recognized(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    line_id: i64,
) -> Result<u64, RevrecRepoError> {
    let result = sqlx::query(
        r#"
        UPDATE revrec_schedule_lines
        SET recognized = true, recognized_at = NOW()
        WHERE id = $1 AND recognized = false
        "#,
    )
    .bind(line_id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Get cumulative recognized amount for a schedule up to and including a period.
pub async fn get_cumulative_recognized(
    pool: &PgPool,
    schedule_id: Uuid,
    up_to_period: &str,
) -> Result<i64, RevrecRepoError> {
    let sum: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT SUM(amount_to_recognize_minor)
        FROM revrec_schedule_lines
        WHERE schedule_id = $1
          AND recognized = true
          AND period <= $2
        "#,
    )
    .bind(schedule_id)
    .bind(up_to_period)
    .fetch_one(pool)
    .await?;
    Ok(sum.unwrap_or(0))
}
