//! Approval workflow service — Guard→Mutation→Outbox atomicity.
//!
//! State machine: draft → submitted → approved / rejected
//! Recall: submitted → draft (employee can pull back before review)
//! Approve locks the period (entries guards check tk_approval_requests.status).

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use crate::events;

const EVT_TIMESHEET_SUBMITTED: &str = "timesheet.submitted";
const EVT_TIMESHEET_APPROVED: &str = "timesheet.approved";
const EVT_TIMESHEET_REJECTED: &str = "timesheet.rejected";
const EVT_TIMESHEET_RECALLED: &str = "timesheet.recalled";

const APPROVAL_COLS: &str = r#"
    id, app_id, employee_id, period_start, period_end, status,
    total_minutes, submitted_at, reviewed_at, reviewer_id,
    reviewer_notes, created_at, updated_at
"#;

// -- Reads ------------------------------------------------------------------

/// Fetch a single approval request by ID.
pub async fn get_approval(
    pool: &PgPool,
    app_id: &str,
    approval_id: Uuid,
) -> Result<ApprovalRequest, ApprovalError> {
    let sql = format!(
        "SELECT {} FROM tk_approval_requests WHERE app_id = $1 AND id = $2",
        APPROVAL_COLS
    );
    sqlx::query_as::<_, ApprovalRequest>(&sql)
        .bind(app_id)
        .bind(approval_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApprovalError::NotFound)
}

/// List approval requests for an employee within a date range.
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

/// Fetch audit trail for an approval request.
pub async fn approval_actions(
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

/// List approvals pending review for a given app (for managers/reviewers).
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

// -- Submit (draft → submitted, or create new + submit) ---------------------

pub async fn submit(
    pool: &PgPool,
    req: &SubmitApprovalRequest,
) -> Result<ApprovalRequest, ApprovalError> {
    validate_submit(req)?;

    let now = Utc::now();
    let event_id = Uuid::new_v4();

    // Calculate total minutes for the period from current entries
    let total: Option<(i64,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(minutes), 0)::BIGINT FROM tk_timesheet_entries \
         WHERE app_id = $1 AND employee_id = $2 \
         AND work_date >= $3 AND work_date <= $4 \
         AND is_current = TRUE AND entry_type != 'void'",
    )
    .bind(&req.app_id)
    .bind(req.employee_id)
    .bind(req.period_start)
    .bind(req.period_end)
    .fetch_optional(pool)
    .await?;
    let total_minutes = total.map(|t| t.0 as i32).unwrap_or(0);

    let mut tx = pool.begin().await?;

    // Upsert: create if not exists, or transition draft/rejected→submitted
    let approval = sqlx::query_as::<_, ApprovalRequest>(&format!(
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
    .bind(&req.app_id)
    .bind(req.employee_id)
    .bind(req.period_start)
    .bind(req.period_end)
    .bind(total_minutes)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // If status didn't change (was already submitted/approved), reject
    if approval.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", approval.status),
            to: "submitted".into(),
        });
    }

    record_action(&mut tx, approval.id, "submit", req.actor_id, None).await?;

    let payload = serde_json::json!({
        "approval_id": approval.id,
        "app_id": req.app_id,
        "employee_id": req.employee_id,
        "period_start": req.period_start,
        "period_end": req.period_end,
        "total_minutes": total_minutes,
    });
    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_TIMESHEET_SUBMITTED,
        "approval_request",
        &approval.id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;
    Ok(approval)
}

// -- Approve (submitted → approved) -----------------------------------------

pub async fn approve(
    pool: &PgPool,
    req: &ReviewApprovalRequest,
) -> Result<ApprovalRequest, ApprovalError> {
    validate_review(req)?;

    let now = Utc::now();
    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let current = fetch_for_update(&mut tx, &req.app_id, req.approval_id).await?;
    if current.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", current.status),
            to: "approved".into(),
        });
    }

    let approval = sqlx::query_as::<_, ApprovalRequest>(&format!(
        "UPDATE tk_approval_requests \
         SET status = 'approved', reviewed_at = $1, reviewer_id = $2, \
             reviewer_notes = $3, updated_at = $1 \
         WHERE id = $4 RETURNING {}",
        APPROVAL_COLS
    ))
    .bind(now)
    .bind(req.actor_id)
    .bind(req.notes.as_deref())
    .bind(req.approval_id)
    .fetch_one(&mut *tx)
    .await?;

    record_action(
        &mut tx,
        approval.id,
        "approve",
        req.actor_id,
        req.notes.as_deref(),
    )
    .await?;

    let payload = serde_json::json!({
        "approval_id": approval.id,
        "app_id": approval.app_id,
        "employee_id": approval.employee_id,
        "period_start": approval.period_start,
        "period_end": approval.period_end,
        "reviewer_id": req.actor_id,
        "total_minutes": approval.total_minutes,
    });
    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_TIMESHEET_APPROVED,
        "approval_request",
        &approval.id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;
    Ok(approval)
}

// -- Reject (submitted → rejected) ------------------------------------------

pub async fn reject(
    pool: &PgPool,
    req: &ReviewApprovalRequest,
) -> Result<ApprovalRequest, ApprovalError> {
    validate_review(req)?;

    let now = Utc::now();
    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let current = fetch_for_update(&mut tx, &req.app_id, req.approval_id).await?;
    if current.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", current.status),
            to: "rejected".into(),
        });
    }

    let approval = sqlx::query_as::<_, ApprovalRequest>(&format!(
        "UPDATE tk_approval_requests \
         SET status = 'rejected', reviewed_at = $1, reviewer_id = $2, \
             reviewer_notes = $3, updated_at = $1 \
         WHERE id = $4 RETURNING {}",
        APPROVAL_COLS
    ))
    .bind(now)
    .bind(req.actor_id)
    .bind(req.notes.as_deref())
    .bind(req.approval_id)
    .fetch_one(&mut *tx)
    .await?;

    record_action(
        &mut tx,
        approval.id,
        "reject",
        req.actor_id,
        req.notes.as_deref(),
    )
    .await?;

    let payload = serde_json::json!({
        "approval_id": approval.id,
        "app_id": approval.app_id,
        "employee_id": approval.employee_id,
        "period_start": approval.period_start,
        "period_end": approval.period_end,
        "reviewer_id": req.actor_id,
        "notes": req.notes,
    });
    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_TIMESHEET_REJECTED,
        "approval_request",
        &approval.id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;
    Ok(approval)
}

// -- Recall (submitted → draft) ---------------------------------------------

pub async fn recall(
    pool: &PgPool,
    req: &RecallApprovalRequest,
) -> Result<ApprovalRequest, ApprovalError> {
    validate_recall(req)?;

    let now = Utc::now();
    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let current = fetch_for_update(&mut tx, &req.app_id, req.approval_id).await?;
    if current.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", current.status),
            to: "draft".into(),
        });
    }

    let approval = sqlx::query_as::<_, ApprovalRequest>(&format!(
        "UPDATE tk_approval_requests \
         SET status = 'draft', submitted_at = NULL, updated_at = $1 \
         WHERE id = $2 RETURNING {}",
        APPROVAL_COLS
    ))
    .bind(now)
    .bind(req.approval_id)
    .fetch_one(&mut *tx)
    .await?;

    record_action(
        &mut tx,
        approval.id,
        "recall",
        req.actor_id,
        req.notes.as_deref(),
    )
    .await?;

    let payload = serde_json::json!({
        "approval_id": approval.id,
        "app_id": approval.app_id,
        "employee_id": approval.employee_id,
        "period_start": approval.period_start,
        "period_end": approval.period_end,
    });
    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_TIMESHEET_RECALLED,
        "approval_request",
        &approval.id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;
    Ok(approval)
}

// -- Internal helpers --------------------------------------------------------

async fn fetch_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ApprovalError::NotFound)
}

async fn record_action(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// -- Validation guards -------------------------------------------------------

fn validate_submit(req: &SubmitApprovalRequest) -> Result<(), ApprovalError> {
    if req.app_id.trim().is_empty() {
        return Err(ApprovalError::Validation("app_id must not be empty".into()));
    }
    if req.period_end < req.period_start {
        return Err(ApprovalError::Validation(
            "period_end must be >= period_start".into(),
        ));
    }
    Ok(())
}

fn validate_review(req: &ReviewApprovalRequest) -> Result<(), ApprovalError> {
    if req.app_id.trim().is_empty() {
        return Err(ApprovalError::Validation("app_id must not be empty".into()));
    }
    Ok(())
}

fn validate_recall(req: &RecallApprovalRequest) -> Result<(), ApprovalError> {
    if req.app_id.trim().is_empty() {
        return Err(ApprovalError::Validation("app_id must not be empty".into()));
    }
    Ok(())
}
