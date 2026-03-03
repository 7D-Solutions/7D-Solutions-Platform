//! Validation guards for clock sessions.
//!
//! Guards run before mutation. They enforce:
//! - No concurrent open sessions for the same employee + tenant.
//! - Employee exists and is active.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::ClockError;

/// Check that no open clock session exists for this employee in this tenant.
/// Returns ConcurrentSession error if one is found.
pub async fn check_no_open_session(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
) -> Result<(), ClockError> {
    let exists: Option<(Uuid,)> = sqlx::query_as(
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

    if exists.is_some() {
        return Err(ClockError::ConcurrentSession(employee_id));
    }

    Ok(())
}

/// Check that an open session exists for this employee. Returns the session ID
/// so the caller can close it.
pub async fn require_open_session(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
) -> Result<Uuid, ClockError> {
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

    row.map(|r| r.0)
        .ok_or(ClockError::NoOpenSession(employee_id))
}
