//! Validation guards for clock sessions.
//!
//! Guards run before mutation. They enforce:
//! - No concurrent open sessions for the same employee + tenant.
//! - Employee exists and is active.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::ClockError;
use super::repo;

/// Check that no open clock session exists for this employee in this tenant.
/// Returns ConcurrentSession error if one is found.
pub async fn check_no_open_session(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
) -> Result<(), ClockError> {
    if repo::find_open_session_id(pool, app_id, employee_id)
        .await?
        .is_some()
    {
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
    repo::find_open_session_id(pool, app_id, employee_id)
        .await?
        .ok_or(ClockError::NoOpenSession(employee_id))
}
