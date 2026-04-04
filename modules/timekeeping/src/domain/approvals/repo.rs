//! Approval repository — SQL layer for tk_approval_requests and tk_approval_actions.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::models::*;

const APPROVAL_COLS: &str = r#"
    id, app_id, employee_id, period_start, period_end, status,
    total_minutes, submitted_at, reviewed_at, reviewer_id,
    reviewer_notes, created_at, updated_at
"#;

// ============================================================================
// Reads (pool-based)
// ============================================================================

pub async fn fetch_approval(
    pool: &PgPool,
    app_id: &str,
    approval_id: Uuid,
) -> Result<Option<ApprovalRequest>, ApprovalError> {
    let sql = format!(
        "SELECT {} FROM tk_approval_requests WHERE app_id = $1 AND id = $2",
        APPROVAL_COLS
    );
    Ok(sqlx::query_as::<_, ApprovalRequest>(&sql)
        .bind(app_id)
        .bind(approval_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn list_approvals(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ApprovalRequest>, ApprovalError> {
    let sql = format!(
        "SELECT {} FROM tk_approval_requests \
         WHERE app_id = $1 AND employee_id = $2 \
         AND period_start >= $3 AND period_end <= $4 \
         ORDER BY period_start",
        APPROVAL_COLS
    );
    Ok(sqlx::query_as::<_, ApprovalRequest>(&sql)
        .bind(app_id)
        .bind(employee_id)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await?)
}

pub async fn fetch_approval_actions(
    pool: &PgPool,
    approval_id: Uuid,
) -> Result<Vec<ApprovalAction>, ApprovalError> {
    Ok(sqlx::query_as::<_, ApprovalAction>(
        "SELECT id, approval_id, action, actor_id, notes, created_at \
         FROM tk_approval_actions WHERE approval_id = $1 ORDER BY created_at ASC",
    )
    .bind(approval_id)
    .fetch_all(pool)
    .await?)
}

pub async fn list_pending_review(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<ApprovalRequest>, ApprovalError> {
    let sql = format!(
        "SELECT {} FROM tk_approval_requests \
         WHERE app_id = $1 AND status = 'submitted' ORDER BY submitted_at",
        APPROVAL_COLS
    );
    Ok(sqlx::query_as::<_, ApprovalRequest>(&sql)
        .bind(app_id)
        .fetch_all(pool)
        .await?)
}

// ============================================================================
// Transaction helpers (conn-based)
// ============================================================================

pub async fn sum_entry_minutes(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<i32, ApprovalError> {
    let total: Option<(i64,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(minutes), 0)::BIGINT FROM tk_timesheet_entries \
         WHERE app_id = $1 AND employee_id = $2 \
         AND work_date >= $3 AND work_date <= $4 \
         AND is_current = TRUE AND entry_type != 'void'",
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_optional(pool)
    .await?;
    Ok(total.map(|t| t.0 as i32).unwrap_or(0))
}

pub async fn upsert_submit(
    conn: &mut PgConnection,
    app_id: &str,
    employee_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    total_minutes: i32,
    now: DateTime<Utc>,
) -> Result<ApprovalRequest, ApprovalError> {
    Ok(sqlx::query_as::<_, ApprovalRequest>(&format!(
        r#"INSERT INTO tk_approval_requests
            (app_id, employee_id, period_start, period_end, status,
             total_minutes, submitted_at, updated_at)
        VALUES ($1, $2, $3, $4, 'submitted', $5, $6, $6)
        ON CONFLICT (app_id, employee_id, period_start, period_end)
        DO UPDATE SET
            status = CASE
                WHEN tk_approval_requests.status IN ('draft', 'rejected')
                THEN 'submitted'::tk_approval_status
                ELSE tk_approval_requests.status END,
            total_minutes = CASE
                WHEN tk_approval_requests.status IN ('draft', 'rejected')
                THEN $5 ELSE tk_approval_requests.total_minutes END,
            submitted_at = CASE
                WHEN tk_approval_requests.status IN ('draft', 'rejected')
                THEN $6 ELSE tk_approval_requests.submitted_at END,
            updated_at = CASE
                WHEN tk_approval_requests.status IN ('draft', 'rejected')
                THEN $6 ELSE tk_approval_requests.updated_at END
        RETURNING {}"#,
        APPROVAL_COLS
    ))
    .bind(app_id)
    .bind(employee_id)
    .bind(period_start)
    .bind(period_end)
    .bind(total_minutes)
    .bind(now)
    .fetch_one(conn)
    .await?)
}

pub async fn update_to_approved(
    conn: &mut PgConnection,
    approval_id: Uuid,
    reviewer_id: Uuid,
    notes: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ApprovalRequest, ApprovalError> {
    Ok(sqlx::query_as::<_, ApprovalRequest>(&format!(
        "UPDATE tk_approval_requests \
         SET status = 'approved', reviewed_at = $1, reviewer_id = $2, \
             reviewer_notes = $3, updated_at = $1 \
         WHERE id = $4 RETURNING {}",
        APPROVAL_COLS
    ))
    .bind(now)
    .bind(reviewer_id)
    .bind(notes)
    .bind(approval_id)
    .fetch_one(conn)
    .await?)
}

pub async fn update_to_rejected(
    conn: &mut PgConnection,
    approval_id: Uuid,
    reviewer_id: Uuid,
    notes: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ApprovalRequest, ApprovalError> {
    Ok(sqlx::query_as::<_, ApprovalRequest>(&format!(
        "UPDATE tk_approval_requests \
         SET status = 'rejected', reviewed_at = $1, reviewer_id = $2, \
             reviewer_notes = $3, updated_at = $1 \
         WHERE id = $4 RETURNING {}",
        APPROVAL_COLS
    ))
    .bind(now)
    .bind(reviewer_id)
    .bind(notes)
    .bind(approval_id)
    .fetch_one(conn)
    .await?)
}

pub async fn update_to_recalled(
    conn: &mut PgConnection,
    approval_id: Uuid,
    now: DateTime<Utc>,
) -> Result<ApprovalRequest, ApprovalError> {
    Ok(sqlx::query_as::<_, ApprovalRequest>(&format!(
        "UPDATE tk_approval_requests \
         SET status = 'draft', submitted_at = NULL, updated_at = $1 \
         WHERE id = $2 RETURNING {}",
        APPROVAL_COLS
    ))
    .bind(now)
    .bind(approval_id)
    .fetch_one(conn)
    .await?)
}

pub async fn fetch_for_update(
    conn: &mut PgConnection,
    app_id: &str,
    approval_id: Uuid,
) -> Result<ApprovalRequest, ApprovalError> {
    let sql = format!(
        "SELECT {} FROM tk_approval_requests WHERE app_id = $1 AND id = $2 FOR UPDATE",
        APPROVAL_COLS
    );
    sqlx::query_as::<_, ApprovalRequest>(&sql)
        .bind(app_id)
        .bind(approval_id)
        .fetch_optional(conn)
        .await?
        .ok_or(ApprovalError::NotFound)
}

pub async fn record_action(
    conn: &mut PgConnection,
    approval_id: Uuid,
    action: &str,
    actor_id: Uuid,
    notes: Option<&str>,
) -> Result<(), ApprovalError> {
    sqlx::query(
        "INSERT INTO tk_approval_actions (approval_id, action, actor_id, notes) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(approval_id)
    .bind(action)
    .bind(actor_id)
    .bind(notes)
    .execute(conn)
    .await?;
    Ok(())
}
