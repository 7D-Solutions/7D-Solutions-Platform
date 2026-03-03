//! Clock in/out service — Guard→Mutation→Outbox atomicity.
//!
//! Protects the invariant: no employee has overlapping open clock sessions.
//! Duration is computed on clock-out as ceiling of elapsed minutes.

use sqlx::PgPool;
use uuid::Uuid;

use super::guards;
use super::models::*;
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
    sqlx::query_as::<_, ClockSession>(
        r#"
        SELECT id, app_id, employee_id, clock_in_at, clock_out_at,
               duration_minutes, status, idempotency_key, created_at, updated_at
        FROM tk_clock_sessions
        WHERE app_id = $1 AND employee_id = $2
        ORDER BY clock_in_at DESC
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .fetch_all(pool)
    .await
    .map_err(ClockError::Database)
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

    let session = sqlx::query_as::<_, ClockSession>(
        r#"
        INSERT INTO tk_clock_sessions (app_id, employee_id, idempotency_key)
        VALUES ($1, $2, $3)
        RETURNING id, app_id, employee_id, clock_in_at, clock_out_at,
                  duration_minutes, status, idempotency_key, created_at, updated_at
        "#,
    )
    .bind(&req.app_id)
    .bind(req.employee_id)
    .bind(req.idempotency_key.as_deref())
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        // The partial unique index will fire 23505 on concurrent open sessions
        // even if the guard check passed (race condition protection).
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.code().as_deref() == Some("23505") {
                return ClockError::ConcurrentSession(req.employee_id);
            }
        }
        ClockError::Database(e)
    })?;

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

    // Close the open session with FOR UPDATE to prevent concurrent clock-outs.
    // Duration = ceiling of elapsed minutes.
    let session = sqlx::query_as::<_, ClockSession>(
        r#"
        UPDATE tk_clock_sessions
        SET clock_out_at = NOW(),
            duration_minutes = CEIL(EXTRACT(EPOCH FROM (NOW() - clock_in_at)) / 60.0)::INTEGER,
            status = 'closed',
            updated_at = NOW()
        WHERE app_id = $1 AND employee_id = $2 AND status = 'open'
        RETURNING id, app_id, employee_id, clock_in_at, clock_out_at,
                  duration_minutes, status, idempotency_key, created_at, updated_at
        "#,
    )
    .bind(&req.app_id)
    .bind(req.employee_id)
    .fetch_optional(&mut *tx)
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
