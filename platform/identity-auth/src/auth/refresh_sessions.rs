//! Refresh-session store for the HttpOnly-cookie refresh flow.
//!
//! Unlike the legacy `refresh_tokens` table (simple fixed-expiry, body-based
//! refresh), `refresh_sessions` tracks sliding-expiry sessions:
//!
//!   - `last_used_at` is updated on every refresh
//!   - `expires_at` slides to `NOW() + REFRESH_IDLE_MINUTES` on every refresh
//!   - `absolute_expires_at` is fixed at `issued_at + REFRESH_ABSOLUTE_MAX_DAYS`
//!   - A session is valid iff all three hold:
//!       · `revoked_at IS NULL`
//!       · `expires_at > NOW()`          (idle check)
//!       · `absolute_expires_at > NOW()` (hard max)
//!
//! Tokens are hashed with SHA-256 before storage; the raw opaque token lives
//! only in the HttpOnly `refresh` cookie delivered to the browser.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use super::refresh::{generate_refresh_token, hash_refresh_token};

/// A single refresh session row, as returned to callers of GET /api/auth/sessions.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RefreshSession {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    #[schema(value_type = Object)]
    pub device_info: Value,
    pub issued_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// Result of a successful refresh: rotated raw token + refreshed session timing.
#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub raw_token: String,
    pub expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// Why a `refresh_sessions` row may be unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionValidationError {
    NotFound,
    Revoked,
    IdleExpired,
    AbsoluteExpired,
}

impl SessionValidationError {
    pub fn as_code(self) -> &'static str {
        match self {
            SessionValidationError::NotFound => "not_found",
            SessionValidationError::Revoked => "revoked",
            SessionValidationError::IdleExpired => "idle_expired",
            SessionValidationError::AbsoluteExpired => "absolute_expired",
        }
    }
}

/// Create a new refresh session and return the raw opaque token (only exposed here —
/// the hash is what lives in the DB). Used by the login handler after a successful
/// password verification.
pub async fn create_session(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    device_info: Value,
    idle_minutes: i64,
    absolute_max_days: i64,
) -> Result<(Uuid, String, DateTime<Utc>, DateTime<Utc>), sqlx::Error> {
    let raw = generate_refresh_token();
    let hash = hash_refresh_token(&raw);

    let now = Utc::now();
    let expires_at = now + Duration::minutes(idle_minutes);
    let absolute_expires_at = now + Duration::days(absolute_max_days);

    let row = sqlx::query(
        r#"
        INSERT INTO refresh_sessions (
            tenant_id, user_id, token_hash, device_info,
            issued_at, last_used_at, expires_at, absolute_expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $5, $6, $7)
        RETURNING session_id
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&hash)
    .bind(&device_info)
    .bind(now)
    .bind(expires_at)
    .bind(absolute_expires_at)
    .fetch_one(&mut **tx)
    .await?;

    let session_id: Uuid = row.get("session_id");
    Ok((session_id, raw, expires_at, absolute_expires_at))
}

/// Look up a session by raw token (hashes internally) and return validation state.
///
/// Intended for the refresh handler's initial lookup. Callers must hold the
/// transaction and perform rotation inside it.
pub async fn find_and_validate(
    tx: &mut Transaction<'_, Postgres>,
    raw_token: &str,
) -> Result<Result<(Uuid, Uuid, Uuid, DateTime<Utc>), SessionValidationError>, sqlx::Error> {
    let hash = hash_refresh_token(raw_token);

    let row = sqlx::query(
        r#"
        SELECT session_id, tenant_id, user_id,
               issued_at, last_used_at, expires_at, absolute_expires_at, revoked_at
        FROM refresh_sessions
        WHERE token_hash = $1
        FOR UPDATE
        "#,
    )
    .bind(&hash)
    .fetch_optional(&mut **tx)
    .await?;

    let row = match row {
        Some(r) => r,
        None => return Ok(Err(SessionValidationError::NotFound)),
    };

    let session_id: Uuid = row.get("session_id");
    let tenant_id: Uuid = row.get("tenant_id");
    let user_id: Uuid = row.get("user_id");
    let expires_at: DateTime<Utc> = row.get("expires_at");
    let absolute_expires_at: DateTime<Utc> = row.get("absolute_expires_at");
    let revoked_at: Option<DateTime<Utc>> = row.get("revoked_at");

    if revoked_at.is_some() {
        return Ok(Err(SessionValidationError::Revoked));
    }

    let now = Utc::now();
    if absolute_expires_at <= now {
        return Ok(Err(SessionValidationError::AbsoluteExpired));
    }
    if expires_at <= now {
        return Ok(Err(SessionValidationError::IdleExpired));
    }

    Ok(Ok((session_id, tenant_id, user_id, absolute_expires_at)))
}

/// Rotate a valid session: revoke the presented session row, insert a new session
/// for the same user/tenant with a fresh opaque token and sliding expiry. This is
/// the atomic primitive behind POST /api/auth/refresh.
///
/// `absolute_expires_at` is preserved from the original session so that sliding
/// a long-active session does not let it outlive the hard cap.
pub async fn rotate(
    tx: &mut Transaction<'_, Postgres>,
    old_session_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    original_absolute_expires_at: DateTime<Utc>,
    idle_minutes: i64,
    device_info: Value,
) -> Result<RefreshResult, sqlx::Error> {
    // Revoke old row (rotation).
    sqlx::query(
        r#"
        UPDATE refresh_sessions
        SET revoked_at = NOW(),
            revocation_reason = COALESCE(revocation_reason, 'rotated'),
            last_used_at = NOW()
        WHERE session_id = $1
        "#,
    )
    .bind(old_session_id)
    .execute(&mut **tx)
    .await?;

    // Insert new row with sliding expiry; absolute_expires_at capped at the
    // original hard maximum.
    let raw = generate_refresh_token();
    let hash = hash_refresh_token(&raw);

    let now = Utc::now();
    let mut expires_at = now + Duration::minutes(idle_minutes);
    if expires_at > original_absolute_expires_at {
        expires_at = original_absolute_expires_at;
    }

    let row = sqlx::query(
        r#"
        INSERT INTO refresh_sessions (
            tenant_id, user_id, token_hash, device_info,
            issued_at, last_used_at, expires_at, absolute_expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $5, $6, $7)
        RETURNING session_id
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&hash)
    .bind(&device_info)
    .bind(now)
    .bind(expires_at)
    .bind(original_absolute_expires_at)
    .fetch_one(&mut **tx)
    .await?;

    let session_id: Uuid = row.get("session_id");
    Ok(RefreshResult {
        session_id,
        tenant_id,
        user_id,
        raw_token: raw,
        expires_at,
        absolute_expires_at: original_absolute_expires_at,
    })
}

/// Revoke all live sessions sharing a token hash (single-use revocation in the
/// logout path; typically one row).
pub async fn revoke_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        r#"
        UPDATE refresh_sessions
        SET revoked_at = NOW(),
            revocation_reason = COALESCE(revocation_reason, $2)
        WHERE token_hash = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(token_hash)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Replay detection: a previously rotated (revoked) token was presented again.
/// Kill every live session for the user so the attacker cannot keep moving.
pub async fn revoke_all_for_user(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        r#"
        UPDATE refresh_sessions
        SET revoked_at = NOW(),
            revocation_reason = COALESCE(revocation_reason, $3)
        WHERE tenant_id = $1 AND user_id = $2 AND revoked_at IS NULL
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected())
}

/// List active (not revoked, within idle + absolute windows) sessions for a user.
pub async fn list_active_for_user(
    pool: &PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<RefreshSession>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT session_id, tenant_id, user_id, device_info,
               issued_at, last_used_at, expires_at, absolute_expires_at
        FROM refresh_sessions
        WHERE tenant_id = $1
          AND user_id = $2
          AND revoked_at IS NULL
          AND expires_at > NOW()
          AND absolute_expires_at > NOW()
        ORDER BY issued_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut sessions = Vec::with_capacity(rows.len());
    for r in rows {
        sessions.push(RefreshSession {
            session_id: r.get("session_id"),
            tenant_id: r.get("tenant_id"),
            user_id: r.get("user_id"),
            device_info: r.get("device_info"),
            issued_at: r.get("issued_at"),
            last_used_at: r.get("last_used_at"),
            expires_at: r.get("expires_at"),
            absolute_expires_at: r.get("absolute_expires_at"),
        });
    }
    Ok(sessions)
}

/// Revoke a specific session. Returns the number of rows affected (0 if the
/// session did not exist, did not belong to the caller, or was already revoked).
pub async fn revoke_by_id(
    pool: &PgPool,
    session_id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        r#"
        UPDATE refresh_sessions
        SET revoked_at = NOW(),
            revocation_reason = COALESCE(revocation_reason, $4)
        WHERE session_id = $1
          AND tenant_id = $2
          AND user_id = $3
          AND revoked_at IS NULL
        "#,
    )
    .bind(session_id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
