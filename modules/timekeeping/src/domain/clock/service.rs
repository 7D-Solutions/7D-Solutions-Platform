//! Clock in/out service — Guard→Mutation→Outbox atomicity.
//!
//! Protects the invariant: no employee has overlapping open clock sessions.
//! Duration is computed on clock-out as ceiling of elapsed minutes.

use sqlx::PgPool;
use uuid::Uuid;

use super::guards;
use super::models::*;
use super::repo;
use crate::events;

const EVT_CLOCK_IN: &str = "clock_session.clocked_in";
const EVT_CLOCK_OUT: &str = "clock_session.clocked_out";

// ============================================================================
// Reads
// ============================================================================

/// List clock sessions for an employee, scoped to tenant.
pub async fn list_sessions(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
) -> Result<Vec<ClockSession>, ClockError> {
    repo::list_sessions(pool, app_id, employee_id).await
}

// ============================================================================
// Clock In
// ============================================================================

pub async fn clock_in(pool: &PgPool, req: &ClockInRequest) -> Result<ClockSession, ClockError> {
    // Guard: validate input
    req.validate()?;

    // Guard: idempotency check
    if let Some(ref key) = req.idempotency_key {
        if let Some((body, code)) = events::check_idempotency(pool, &req.app_id, key).await? {
            return Err(ClockError::IdempotentReplay {
                status_code: code as u16,
                body,
            });
        }
    }

    // Guard: no concurrent open session
    guards::check_no_open_session(pool, &req.app_id, req.employee_id).await?;

    // Mutation + Outbox (atomic)
    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let session = repo::insert_clock_session(
        &mut *tx,
        &req.app_id,
        req.employee_id,
        req.idempotency_key.as_deref(),
    )
    .await?;

    let payload = serde_json::json!({
        "session_id": session.id,
        "app_id": req.app_id,
        "employee_id": req.employee_id,
        "clock_in_at": session.clock_in_at,
    });

    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_CLOCK_IN,
        "clock_session",
        &session.id.to_string(),
        &payload,
    )
    .await?;

    if let Some(ref key) = req.idempotency_key {
        events::record_idempotency(&mut tx, &req.app_id, key, &session, 201).await?;
    }

    tx.commit().await?;

    Ok(session)
}

// ============================================================================
// Clock Out
// ============================================================================

pub async fn clock_out(pool: &PgPool, req: &ClockOutRequest) -> Result<ClockSession, ClockError> {
    // Guard: validate input
    req.validate()?;

    // Guard: idempotency check
    if let Some(ref key) = req.idempotency_key {
        if let Some((body, code)) = events::check_idempotency(pool, &req.app_id, key).await? {
            return Err(ClockError::IdempotentReplay {
                status_code: code as u16,
                body,
            });
        }
    }

    // Guard: must have an open session
    let _session_id = guards::require_open_session(pool, &req.app_id, req.employee_id).await?;

    // Mutation + Outbox (atomic)
    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    // Close the open session with duration computation.
    let session = repo::close_clock_session(&mut *tx, &req.app_id, req.employee_id)
        .await?
        .ok_or(ClockError::NoOpenSession(req.employee_id))?;

    let payload = serde_json::json!({
        "session_id": session.id,
        "app_id": req.app_id,
        "employee_id": req.employee_id,
        "clock_in_at": session.clock_in_at,
        "clock_out_at": session.clock_out_at,
        "duration_minutes": session.duration_minutes,
    });

    events::enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_CLOCK_OUT,
        "clock_session",
        &session.id.to_string(),
        &payload,
    )
    .await?;

    if let Some(ref key) = req.idempotency_key {
        events::record_idempotency(&mut tx, &req.app_id, key, &session, 200).await?;
    }

    tx.commit().await?;

    Ok(session)
}
