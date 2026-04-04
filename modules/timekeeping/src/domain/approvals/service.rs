//! Approval workflow service — Guard→Mutation→Outbox atomicity.
//!
//! State machine: draft → submitted → approved / rejected
//! Recall: submitted → draft (employee can pull back before review)
//! Approve locks the period (entries guards check tk_approval_requests.status).

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use super::repo;
use crate::events;

const EVT_TIMESHEET_SUBMITTED: &str = "timesheet.submitted";
const EVT_TIMESHEET_APPROVED: &str = "timesheet.approved";
const EVT_TIMESHEET_REJECTED: &str = "timesheet.rejected";
const EVT_TIMESHEET_RECALLED: &str = "timesheet.recalled";

// -- Reads ------------------------------------------------------------------

/// Fetch a single approval request by ID.
pub async fn get_approval(
    pool: &PgPool,
    app_id: &str,
    approval_id: Uuid,
) -> Result<ApprovalRequest, ApprovalError> {
    repo::fetch_approval(pool, app_id, approval_id)
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
    repo::list_approvals(pool, app_id, employee_id, from, to).await
}

/// Fetch audit trail for an approval request.
pub async fn approval_actions(
    pool: &PgPool,
    approval_id: Uuid,
) -> Result<Vec<ApprovalAction>, ApprovalError> {
    repo::fetch_approval_actions(pool, approval_id).await
}

/// List approvals pending review for a given app (for managers/reviewers).
pub async fn list_pending_review(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<ApprovalRequest>, ApprovalError> {
    repo::list_pending_review(pool, app_id).await
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
    let total_minutes = repo::sum_entry_minutes(
        pool,
        &req.app_id,
        req.employee_id,
        req.period_start,
        req.period_end,
    )
    .await?;

    let mut tx = pool.begin().await?;

    // Upsert: create if not exists, or transition draft/rejected→submitted
    let approval = repo::upsert_submit(
        &mut *tx,
        &req.app_id,
        req.employee_id,
        req.period_start,
        req.period_end,
        total_minutes,
        now,
    )
    .await?;

    // If status didn't change (was already submitted/approved), reject
    if approval.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", approval.status),
            to: "submitted".into(),
        });
    }

    repo::record_action(&mut *tx, approval.id, "submit", req.actor_id, None).await?;

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

    let current = repo::fetch_for_update(&mut *tx, &req.app_id, req.approval_id).await?;
    if current.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", current.status),
            to: "approved".into(),
        });
    }

    let approval = repo::update_to_approved(
        &mut *tx,
        req.approval_id,
        req.actor_id,
        req.notes.as_deref(),
        now,
    )
    .await?;

    repo::record_action(
        &mut *tx,
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

    let current = repo::fetch_for_update(&mut *tx, &req.app_id, req.approval_id).await?;
    if current.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", current.status),
            to: "rejected".into(),
        });
    }

    let approval = repo::update_to_rejected(
        &mut *tx,
        req.approval_id,
        req.actor_id,
        req.notes.as_deref(),
        now,
    )
    .await?;

    repo::record_action(
        &mut *tx,
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

    let current = repo::fetch_for_update(&mut *tx, &req.app_id, req.approval_id).await?;
    if current.status != ApprovalStatus::Submitted {
        return Err(ApprovalError::InvalidTransition {
            from: format!("{:?}", current.status),
            to: "draft".into(),
        });
    }

    let approval = repo::update_to_recalled(&mut *tx, req.approval_id, now).await?;

    repo::record_action(
        &mut *tx,
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
