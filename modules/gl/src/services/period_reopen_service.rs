//! Period Reopen Service
//!
//! Controlled reopen of closed accounting periods with immutable audit trail.
//! Reopen is exceptional and requires explicit request → approval → execution.
//! All operations are append-only; the period_reopen_requests table is immutable.

use crate::repos::outbox_repo;
use crate::services::period_close_validation::PeriodCloseError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Result of a reopen request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReopenRequestResult {
    pub request_id: Uuid,
    pub period_id: Uuid,
    pub tenant_id: String,
    pub status: String,
    pub prior_close_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Result of approving/rejecting a reopen
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReopenDecisionResult {
    pub request_id: Uuid,
    pub period_id: Uuid,
    pub tenant_id: String,
    pub status: String,
    pub decided_by: String,
    pub decided_at: DateTime<Utc>,
}

/// Row type for reopen request queries
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReopenRequestRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub requested_by: String,
    pub reason: String,
    pub prior_close_hash: String,
    pub status: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub rejected_by: Option<String>,
    pub rejected_at: Option<DateTime<Utc>>,
    pub reject_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Row type for period close fields query
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct PeriodCloseFields {
    id: Uuid,
    closed_at: Option<DateTime<Utc>>,
    close_hash: Option<String>,
}

/// Request a period reopen.
///
/// Guards:
/// - Period must exist and be closed (closed_at IS NOT NULL)
/// - prior_close_hash is captured from the current close_hash (proves which sealed state)
/// - No pending reopen requests for this period (prevent duplicate requests)
///
/// Creates an append-only row in period_reopen_requests with status='requested'.
pub async fn request_reopen(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    requested_by: &str,
    reason: &str,
) -> Result<ReopenRequestResult, PeriodCloseError> {
    let mut tx = pool.begin().await?;

    // Guard: period must be closed
    let period = sqlx::query_as::<_, PeriodCloseFields>(
        r#"
        SELECT id, closed_at, close_hash
        FROM accounting_periods
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PeriodCloseError::PeriodNotFound(period_id))?;

    if period.closed_at.is_none() {
        return Err(PeriodCloseError::ValidationFailed(
            "Period is not closed — cannot request reopen".to_string(),
        ));
    }

    let prior_close_hash = period.close_hash.unwrap_or_default();

    // Guard: no pending reopen requests
    let pending_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*) FROM period_reopen_requests
        WHERE tenant_id = $1 AND period_id = $2 AND status = 'requested'
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&mut *tx)
    .await?;

    if pending_count > 0 {
        return Err(PeriodCloseError::ValidationFailed(
            "A pending reopen request already exists for this period".to_string(),
        ));
    }

    // Insert reopen request (append-only)
    let request_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO period_reopen_requests
            (id, tenant_id, period_id, requested_by, reason, prior_close_hash, status, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'requested', $7)
        "#,
    )
    .bind(request_id)
    .bind(tenant_id)
    .bind(period_id)
    .bind(requested_by)
    .bind(reason)
    .bind(&prior_close_hash)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(ReopenRequestResult {
        request_id,
        period_id,
        tenant_id: tenant_id.to_string(),
        status: "requested".to_string(),
        prior_close_hash,
        created_at: now,
    })
}

/// Approve a reopen request and execute the reopen.
///
/// Guards:
/// - Request must exist and be in 'requested' status
/// - Period must still be closed (concurrent close/reopen protection)
///
/// Atomically:
/// 1. Updates request status to 'approved'
/// 2. Clears closed_at, closed_by, close_reason, close_hash on accounting_periods
/// 3. Increments reopen_count, sets last_reopened_at
/// 4. Emits gl.period.reopened outbox event
pub async fn approve_reopen(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    request_id: Uuid,
    approved_by: &str,
) -> Result<ReopenDecisionResult, PeriodCloseError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // Lock period row
    let period = sqlx::query_as::<_, PeriodCloseFields>(
        r#"
        SELECT id, closed_at, close_hash
        FROM accounting_periods
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PeriodCloseError::PeriodNotFound(period_id))?;

    if period.closed_at.is_none() {
        return Err(PeriodCloseError::ValidationFailed(
            "Period is no longer closed — cannot approve reopen".to_string(),
        ));
    }

    // Validate request exists and is pending
    let request_status = sqlx::query_scalar::<_, String>(
        r#"
        SELECT status FROM period_reopen_requests
        WHERE id = $1 AND tenant_id = $2 AND period_id = $3
        FOR UPDATE
        "#,
    )
    .bind(request_id)
    .bind(tenant_id)
    .bind(period_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        PeriodCloseError::ValidationFailed(format!("Reopen request {} not found", request_id))
    })?;

    if request_status != "requested" {
        return Err(PeriodCloseError::ValidationFailed(format!(
            "Reopen request is '{}', expected 'requested'",
            request_status
        )));
    }

    // 1. Approve the request
    sqlx::query(
        r#"
        UPDATE period_reopen_requests
        SET status = 'approved', approved_by = $1, approved_at = $2
        WHERE id = $3
        "#,
    )
    .bind(approved_by)
    .bind(now)
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    // 2. Clear close fields on accounting_periods, increment reopen_count
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = NULL,
            closed_by = NULL,
            close_reason = NULL,
            close_hash = NULL,
            close_requested_at = NULL,
            reopen_count = reopen_count + 1,
            last_reopened_at = $1
        WHERE id = $2 AND tenant_id = $3
        "#,
    )
    .bind(now)
    .bind(period_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    // 3. Emit gl.period.reopened outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "period_id": period_id,
        "tenant_id": tenant_id,
        "request_id": request_id,
        "prior_close_hash": period.close_hash,
        "approved_by": approved_by,
        "reopened_at": now.to_rfc3339(),
    });

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        "gl.period.reopened",
        "accounting_period",
        &period_id.to_string(),
        payload,
        "ADMINISTRATIVE",
    )
    .await?;

    tx.commit().await?;

    Ok(ReopenDecisionResult {
        request_id,
        period_id,
        tenant_id: tenant_id.to_string(),
        status: "approved".to_string(),
        decided_by: approved_by.to_string(),
        decided_at: now,
    })
}

/// Reject a reopen request.
///
/// Guards:
/// - Request must exist and be in 'requested' status
///
/// Marks the request as 'rejected' with reason. Period state is unchanged.
pub async fn reject_reopen(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    request_id: Uuid,
    rejected_by: &str,
    reject_reason: &str,
) -> Result<ReopenDecisionResult, PeriodCloseError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // Validate request exists and is pending
    let request_status = sqlx::query_scalar::<_, String>(
        r#"
        SELECT status FROM period_reopen_requests
        WHERE id = $1 AND tenant_id = $2 AND period_id = $3
        FOR UPDATE
        "#,
    )
    .bind(request_id)
    .bind(tenant_id)
    .bind(period_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        PeriodCloseError::ValidationFailed(format!("Reopen request {} not found", request_id))
    })?;

    if request_status != "requested" {
        return Err(PeriodCloseError::ValidationFailed(format!(
            "Reopen request is '{}', expected 'requested'",
            request_status
        )));
    }

    sqlx::query(
        r#"
        UPDATE period_reopen_requests
        SET status = 'rejected', rejected_by = $1, rejected_at = $2, reject_reason = $3
        WHERE id = $4
        "#,
    )
    .bind(rejected_by)
    .bind(now)
    .bind(reject_reason)
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(ReopenDecisionResult {
        request_id,
        period_id,
        tenant_id: tenant_id.to_string(),
        status: "rejected".to_string(),
        decided_by: rejected_by.to_string(),
        decided_at: now,
    })
}

/// List reopen requests for a period (audit trail query).
pub async fn list_reopen_requests(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Vec<ReopenRequestRow>, PeriodCloseError> {
    let rows = sqlx::query_as::<_, ReopenRequestRow>(
        r#"
        SELECT id, tenant_id, period_id, requested_by, reason, prior_close_hash,
               status, approved_by, approved_at, rejected_by, rejected_at,
               reject_reason, created_at
        FROM period_reopen_requests
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
