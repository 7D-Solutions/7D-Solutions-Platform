//! Clock session repository — SQL layer for tk_clock_sessions.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::models::*;

// ============================================================================
// Reads
// ============================================================================

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
// Guard queries
// ============================================================================

/// Returns Some(session_id) if an open session exists.
pub async fn find_open_session_id(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
) -> Result<Option<Uuid>, ClockError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id FROM tk_clock_sessions
        WHERE app_id = $1 AND employee_id = $2 AND status = 'open'
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .fetch_optional(pool)
    .await
    .map_err(ClockError::Database)?;
    Ok(row.map(|r| r.0))
}

// ============================================================================
// Writes
// ============================================================================

pub async fn insert_clock_session(
    conn: &mut PgConnection,
    app_id: &str,
    employee_id: Uuid,
    idempotency_key: Option<&str>,
) -> Result<ClockSession, ClockError> {
    sqlx::query_as::<_, ClockSession>(
        r#"
        INSERT INTO tk_clock_sessions (app_id, employee_id, idempotency_key)
        VALUES ($1, $2, $3)
        RETURNING id, app_id, employee_id, clock_in_at, clock_out_at,
                  duration_minutes, status, idempotency_key, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(idempotency_key)
    .fetch_one(conn)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.code().as_deref() == Some("23505") {
                return ClockError::ConcurrentSession(employee_id);
            }
        }
        ClockError::Database(e)
    })
}

pub async fn close_clock_session(
    conn: &mut PgConnection,
    app_id: &str,
    employee_id: Uuid,
) -> Result<Option<ClockSession>, ClockError> {
    Ok(sqlx::query_as::<_, ClockSession>(
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
    .bind(app_id)
    .bind(employee_id)
    .fetch_optional(conn)
    .await?)
}
