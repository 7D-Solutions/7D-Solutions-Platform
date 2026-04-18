//! Integration tests for sliding-expiry refresh sessions (bd-b9nof).
//!
//! Exercises the `refresh_sessions` domain module against a real Postgres.
//! No mocks, no stubs. Run with `./scripts/cargo-slot.sh test -p auth-rs`.

use auth_rs::auth::cookies;
use auth_rs::auth::refresh::hash_refresh_token;
use auth_rs::auth::refresh_sessions::{self, SessionValidationError};
use axum::http::{HeaderMap, HeaderValue};
use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to auth test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

const IDLE_MINUTES: i64 = 480;
const ABSOLUTE_MAX_DAYS: i64 = 30;

// ---------------------------------------------------------------------------
// create_session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_session_stores_row_with_sliding_and_absolute_windows() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (session_id, raw, expires_at, absolute_expires_at) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({"ua": "integration-test"}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create session");
    tx.commit().await.expect("commit");

    assert!(!raw.is_empty(), "raw token must be returned");
    assert!(expires_at > Utc::now(), "sliding expiry must be in future");
    assert!(absolute_expires_at > expires_at, "absolute > sliding");

    let hash = hash_refresh_token(&raw);
    let row = sqlx::query(
        "SELECT user_id, tenant_id, device_info, revoked_at FROM refresh_sessions WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("fetch row");
    let stored_uid: Uuid = row.get("user_id");
    let stored_tid: Uuid = row.get("tenant_id");
    let device_info: serde_json::Value = row.get("device_info");
    let revoked_at: Option<chrono::DateTime<Utc>> = row.get("revoked_at");
    assert_eq!(stored_uid, user_id);
    assert_eq!(stored_tid, tenant_id);
    assert!(revoked_at.is_none());
    assert_eq!(device_info["ua"], "integration-test");

    // Token must be stored as SHA-256 hash, not plaintext.
    let stored_hash: String = sqlx::query_scalar("SELECT token_hash FROM refresh_sessions WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("fetch hash");
    assert_eq!(stored_hash, hash);
    assert_ne!(stored_hash, raw, "raw token must not be stored");
}

// ---------------------------------------------------------------------------
// find_and_validate — happy + unhappy paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_and_validate_accepts_fresh_session() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (_sid, raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    let mut tx2 = pool.begin().await.expect("begin2");
    let result = refresh_sessions::find_and_validate(&mut tx2, &raw)
        .await
        .expect("validate query");
    tx2.rollback().await.ok();

    let (_found_sid, found_tid, found_uid, _abs_ex) = result.expect("session must validate");
    assert_eq!(found_tid, tenant_id);
    assert_eq!(found_uid, user_id);
}

#[tokio::test]
async fn find_and_validate_rejects_revoked_session() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (session_id, raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    sqlx::query("UPDATE refresh_sessions SET revoked_at = NOW() WHERE session_id = $1")
        .bind(session_id)
        .execute(&pool)
        .await
        .expect("revoke");

    let mut tx2 = pool.begin().await.expect("begin");
    let result = refresh_sessions::find_and_validate(&mut tx2, &raw)
        .await
        .expect("validate query");
    tx2.rollback().await.ok();
    assert_eq!(result.err(), Some(SessionValidationError::Revoked));
}

#[tokio::test]
async fn find_and_validate_rejects_idle_expired_session() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (session_id, raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    // Move the idle expiry into the past (simulate an idle user).
    sqlx::query("UPDATE refresh_sessions SET expires_at = NOW() - INTERVAL '1 minute' WHERE session_id = $1")
        .bind(session_id)
        .execute(&pool)
        .await
        .expect("expire");

    let mut tx2 = pool.begin().await.expect("begin");
    let result = refresh_sessions::find_and_validate(&mut tx2, &raw)
        .await
        .expect("validate");
    tx2.rollback().await.ok();
    assert_eq!(result.err(), Some(SessionValidationError::IdleExpired));
}

#[tokio::test]
async fn find_and_validate_rejects_absolute_expired_session() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (session_id, raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    // Push the hard max into the past. Absolute check must fire before idle.
    sqlx::query("UPDATE refresh_sessions SET absolute_expires_at = NOW() - INTERVAL '1 minute' WHERE session_id = $1")
        .bind(session_id)
        .execute(&pool)
        .await
        .expect("expire abs");

    let mut tx2 = pool.begin().await.expect("begin");
    let result = refresh_sessions::find_and_validate(&mut tx2, &raw)
        .await
        .expect("validate");
    tx2.rollback().await.ok();
    assert_eq!(result.err(), Some(SessionValidationError::AbsoluteExpired));
}

#[tokio::test]
async fn find_and_validate_returns_not_found_for_unknown_token() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin");
    let result = refresh_sessions::find_and_validate(&mut tx, "garbage-token-xyz")
        .await
        .expect("validate");
    tx.rollback().await.ok();
    assert_eq!(result.err(), Some(SessionValidationError::NotFound));
}

// ---------------------------------------------------------------------------
// rotate — old row revoked, new row issued, absolute cap preserved
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rotate_revokes_old_and_issues_new_with_absolute_cap_preserved() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (old_sid, _old_raw, _ex, absolute_expires_at) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    let rotated = refresh_sessions::rotate(
        &mut tx,
        old_sid,
        tenant_id,
        user_id,
        absolute_expires_at,
        IDLE_MINUTES,
        serde_json::json!({}),
    )
    .await
    .expect("rotate");
    tx.commit().await.expect("commit");

    // Old row revoked.
    let old_revoked: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM refresh_sessions WHERE session_id = $1")
            .bind(old_sid)
            .fetch_one(&pool)
            .await
            .expect("old row");
    assert!(old_revoked.is_some(), "old session must be revoked");

    // New row has same absolute cap (sliding doesn't outlive the hard max).
    let new_abs: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT absolute_expires_at FROM refresh_sessions WHERE session_id = $1")
            .bind(rotated.session_id)
            .fetch_one(&pool)
            .await
            .expect("new row");
    assert_eq!(new_abs, absolute_expires_at);
}

#[tokio::test]
async fn rotate_caps_expires_at_when_idle_window_would_exceed_absolute() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    // Seed a session whose absolute cap is very close to now (2 minutes out)
    // while idle window is 8h — the rotated expires_at must be capped at
    // absolute_expires_at, not `now + 8h`.
    let mut tx = pool.begin().await.expect("begin");
    let (old_sid, _raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    let near_abs: chrono::DateTime<Utc> = Utc::now() + chrono::Duration::minutes(2);
    sqlx::query("UPDATE refresh_sessions SET absolute_expires_at = $1 WHERE session_id = $2")
        .bind(near_abs)
        .bind(old_sid)
        .execute(&pool)
        .await
        .expect("shrink cap");

    let mut tx2 = pool.begin().await.expect("begin2");
    let rotated = refresh_sessions::rotate(
        &mut tx2,
        old_sid,
        tenant_id,
        user_id,
        near_abs,
        IDLE_MINUTES,
        serde_json::json!({}),
    )
    .await
    .expect("rotate");
    tx2.commit().await.expect("commit2");

    assert!(
        rotated.expires_at <= near_abs,
        "sliding expires_at ({}) must not exceed absolute cap ({})",
        rotated.expires_at,
        near_abs
    );
}

// ---------------------------------------------------------------------------
// Replay detection: presenting a revoked (rotated) token must kill the whole
// user's live sessions via revoke_all_for_user.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_all_for_user_kills_all_live_sessions_for_that_user() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let other_user = Uuid::new_v4();

    // Three sessions for the target user.
    for _ in 0..3 {
        let mut tx = pool.begin().await.expect("begin");
        refresh_sessions::create_session(
            &mut tx,
            tenant_id,
            user_id,
            serde_json::json!({}),
            IDLE_MINUTES,
            ABSOLUTE_MAX_DAYS,
        )
        .await
        .expect("create");
        tx.commit().await.expect("commit");
    }

    // One session for an unrelated user — must NOT be touched.
    let mut tx = pool.begin().await.expect("begin");
    let (other_sid, _raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        other_user,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    let mut tx = pool.begin().await.expect("begin");
    let count =
        refresh_sessions::revoke_all_for_user(&mut tx, tenant_id, user_id, "replay_detected")
            .await
            .expect("revoke all");
    tx.commit().await.expect("commit");
    assert_eq!(count, 3);

    let live_for_user: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM refresh_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(live_for_user, 0);

    let other_revoked: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM refresh_sessions WHERE session_id = $1")
            .bind(other_sid)
            .fetch_one(&pool)
            .await
            .expect("other row");
    assert!(other_revoked.is_none(), "other user's session must not be touched");
}

// ---------------------------------------------------------------------------
// list_active_for_user / revoke_by_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_active_excludes_revoked_and_expired_sessions() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    // 1 fresh, 1 revoked, 1 idle-expired
    let mut sids = Vec::new();
    for _ in 0..3 {
        let mut tx = pool.begin().await.expect("begin");
        let (sid, _raw, _ex, _abs) = refresh_sessions::create_session(
            &mut tx,
            tenant_id,
            user_id,
            serde_json::json!({}),
            IDLE_MINUTES,
            ABSOLUTE_MAX_DAYS,
        )
        .await
        .expect("create");
        tx.commit().await.expect("commit");
        sids.push(sid);
    }
    sqlx::query("UPDATE refresh_sessions SET revoked_at = NOW() WHERE session_id = $1")
        .bind(sids[1])
        .execute(&pool)
        .await
        .expect("revoke");
    sqlx::query("UPDATE refresh_sessions SET expires_at = NOW() - INTERVAL '1 minute' WHERE session_id = $1")
        .bind(sids[2])
        .execute(&pool)
        .await
        .expect("idle expire");

    let active = refresh_sessions::list_active_for_user(&pool, tenant_id, user_id)
        .await
        .expect("list active");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, sids[0]);
}

#[tokio::test]
async fn revoke_by_id_only_affects_the_matching_session_for_the_caller() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let different_user = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (sid, _raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    // Wrong caller cannot revoke.
    let n_wrong = refresh_sessions::revoke_by_id(&pool, sid, tenant_id, different_user, "try")
        .await
        .expect("revoke query");
    assert_eq!(n_wrong, 0);

    let revoked: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM refresh_sessions WHERE session_id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .expect("row");
    assert!(revoked.is_none(), "session must NOT be revoked by a stranger");

    // Right caller can.
    let n_right = refresh_sessions::revoke_by_id(&pool, sid, tenant_id, user_id, "me")
        .await
        .expect("revoke");
    assert_eq!(n_right, 1);

    let revoked: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM refresh_sessions WHERE session_id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .expect("row");
    assert!(revoked.is_some(), "session must be revoked by owner");
}

// ---------------------------------------------------------------------------
// revoke_by_token_hash — used by logout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_by_token_hash_marks_the_session_revoked() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin");
    let (sid, raw, _ex, _abs) = refresh_sessions::create_session(
        &mut tx,
        tenant_id,
        user_id,
        serde_json::json!({}),
        IDLE_MINUTES,
        ABSOLUTE_MAX_DAYS,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    let hash = hash_refresh_token(&raw);
    let n = refresh_sessions::revoke_by_token_hash(&pool, &hash, "logout")
        .await
        .expect("revoke");
    assert_eq!(n, 1);

    let reason: Option<String> =
        sqlx::query_scalar("SELECT revocation_reason FROM refresh_sessions WHERE session_id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .expect("row");
    assert_eq!(reason.as_deref(), Some("logout"));
}

// ---------------------------------------------------------------------------
// Cookie helpers
// ---------------------------------------------------------------------------

#[test]
fn cookie_helper_reads_refresh_cookie_from_header() {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        HeaderValue::from_static("foo=bar; refresh=abc123; other=x"),
    );
    assert_eq!(cookies::read_refresh_cookie(&headers).as_deref(), Some("abc123"));
}

#[test]
fn cookie_helper_absent_when_no_refresh_cookie() {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        HeaderValue::from_static("foo=bar"),
    );
    assert!(cookies::read_refresh_cookie(&headers).is_none());
}

#[test]
fn cookie_helper_set_cookie_is_httponly_samesite_lax_path_scoped() {
    let v = cookies::build_set_cookie("TOKEN123", 3600, true);
    assert!(v.contains("refresh=TOKEN123"));
    assert!(v.contains("HttpOnly"));
    assert!(v.contains("SameSite=Lax"));
    assert!(v.contains("Path=/api/auth"));
    assert!(v.contains("Max-Age=3600"));
    assert!(v.contains("Secure"));
}

#[test]
fn cookie_helper_set_cookie_omits_secure_in_non_secure_mode() {
    let v = cookies::build_set_cookie("TOKEN123", 3600, false);
    assert!(!v.contains("Secure"));
}

#[test]
fn cookie_helper_clear_cookie_zeros_max_age() {
    let v = cookies::build_clear_cookie(true);
    assert!(v.contains("Max-Age=0"));
    assert!(v.contains("HttpOnly"));
}
