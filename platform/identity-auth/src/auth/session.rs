use crate::{
    clients::tenant_registry::TenantGate, middleware::tracing::get_trace_id_from_extensions,
};
use axum::{
    extract::{Path, Query, State},
    http::{Extensions, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use event_bus::EventEnvelope;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use super::cookies;
use super::handlers::{err, err_retry_after, ApiErr, AuthState, OkResponse, TokenResponse};
use super::refresh::{generate_refresh_token, hash_refresh_token};
use super::refresh_sessions::{self, RefreshSession, SessionValidationError};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RefreshReq {
    /// Deprecated: the server looks this up from the session row.
    /// Field is accepted for back-compat but ignored; server row is authoritative.
    #[serde(default)]
    pub tenant_id: Option<Uuid>,
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LogoutReq {
    pub tenant_id: Uuid,
    pub refresh_token: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccessTokenResponse {
    pub token_type: &'static str,
    pub access_token: String,
    pub expires_in_seconds: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionsListResponse {
    pub sessions: Vec<RefreshSession>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SessionsQuery {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokeSessionReq {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
}

// ---------------------------------------------------------------------------
// POST /api/auth/refresh — cookie-aware, falls back to body for legacy callers
// ---------------------------------------------------------------------------

#[utoipa::path(post, path = "/api/auth/refresh", tag = "Auth",
    security(()),
    request_body(content = RefreshReq, content_type = "application/json", description = "Body-based refresh (legacy). Omit to use HttpOnly cookie flow."),
    responses(
        (status = 200, description = "Tokens refreshed", body = TokenResponse),
        (status = 401, description = "Invalid or expired refresh token"),
        (status = 403, description = "Tenant suspended"),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "Service busy or seat limit reached"),
    ))]
pub async fn refresh(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    headers: HeaderMap,
    body: Option<Json<RefreshReq>>,
) -> Result<axum::response::Response, ApiErr> {
    // Cookie flow takes priority; body flow remains for legacy callers.
    if let Some(cookie_token) = cookies::read_refresh_cookie(&headers) {
        return refresh_via_cookie(state, extensions, cookie_token).await;
    }

    let Json(req) = body.ok_or_else(|| err(StatusCode::BAD_REQUEST, "refresh token required"))?;
    refresh_via_body(state, extensions, &headers, req).await
}

async fn refresh_via_cookie(
    state: Arc<AuthState>,
    extensions: Extensions,
    raw_token: String,
) -> Result<axum::response::Response, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let token_hash = hash_refresh_token(&raw_token);
    let hash_prefix: String = token_hash.chars().take(12).collect();

    let mut tx = state.db.begin().await.map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let validated = refresh_sessions::find_and_validate(&mut tx, &raw_token)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
        })?;

    let (old_session_id, tenant_id, user_id, absolute_expires_at) = match validated {
        Ok(v) => v,
        Err(SessionValidationError::Revoked) => {
            // Replay: the presented token was a valid session that has since been
            // rotated/revoked. Burn every remaining live session for this user so
            // the attacker cannot keep moving, then return 401.
            if let Some(row) =
                sqlx::query("SELECT tenant_id, user_id FROM refresh_sessions WHERE token_hash = $1")
                    .bind(&token_hash)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?
            {
                let tenant_id: Uuid = row.get("tenant_id");
                let user_id: Uuid = row.get("user_id");
                let _ = refresh_sessions::revoke_all_for_user(
                    &mut tx,
                    tenant_id,
                    user_id,
                    "replay_detected",
                )
                .await;
                tx.commit().await.ok();

                state
                    .metrics
                    .auth_refresh_replay_total
                    .with_label_values(&[&tenant_id.to_string()])
                    .inc();
                tracing::warn!(
                    tenant_id = %tenant_id,
                    user_id = %user_id,
                    trace_id = %trace_id,
                    token_hash_prefix = %hash_prefix,
                    "security.refresh_replay_detected"
                );
            }
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "revoked"])
                .inc();
            return Err(err(StatusCode::UNAUTHORIZED, "refresh_invalid"));
        }
        Err(code) => {
            let _ = tx.rollback().await;
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", code.as_code()])
                .inc();
            return Err(err(StatusCode::UNAUTHORIZED, "refresh_invalid"));
        }
    };

    // Rate limit per (tenant, hash_prefix) — use the same limiter as legacy flow.
    if let Err(wait) = state.keyed_limits.check_refresh(
        &tenant_id.to_string(),
        &hash_prefix,
        state.refresh_per_min_per_token,
    ) {
        let _ = tx.rollback().await;
        state
            .metrics
            .auth_rate_limited_total
            .with_label_values(&["refresh"])
            .inc();
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "rate_limited"])
            .inc();
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
    }

    // Tenant lifecycle gate — same rules as legacy refresh.
    if let Some(client) = &state.tenant_registry {
        match client.get_tenant_gate(tenant_id, &state.metrics).await {
            Ok(TenantGate::Allow) | Ok(TenantGate::DenyNewLogin { .. }) => {}
            Ok(TenantGate::Deny { status }) => {
                let _ = tx.rollback().await;
                state
                    .metrics
                    .auth_tenant_status_denied_total
                    .with_label_values(&[&status])
                    .inc();
                return Err(err(StatusCode::FORBIDDEN, "tenant account inactive"));
            }
            Err(_) => {
                let _ = tx.rollback().await;
                state
                    .metrics
                    .auth_tenant_status_denied_total
                    .with_label_values(&["unavailable"])
                    .inc();
                return Err(err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "tenant status service unavailable",
                ));
            }
        }
    }

    // Rotate: revoke old session row, insert fresh one with sliding expiry.
    let rotated = refresh_sessions::rotate(
        &mut tx,
        old_session_id,
        tenant_id,
        user_id,
        absolute_expires_at,
        state.refresh_idle_minutes,
        serde_json::json!({}),
    )
    .await
    .map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    tx.commit().await.map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    // Mint a new access token with fresh RBAC snapshot.
    let roles = crate::db::rbac::list_roles_for_user(&state.db, tenant_id, user_id)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac roles: {e}"),
            )
        })?
        .into_iter()
        .map(|r| r.name)
        .collect::<Vec<_>>();
    let perms = crate::db::rbac::effective_permissions_for_user(&state.db, tenant_id, user_id)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac perms: {e}"),
            )
        })?;
    let role_snapshot_id = super::jwt::compute_role_snapshot_id(&roles);

    let access = state
        .jwt
        .sign_access_token_enriched(
            tenant_id,
            user_id,
            roles,
            perms,
            super::jwt::actor_type::USER,
            state.access_ttl_minutes,
            Some(rotated.session_id),
            Some(role_snapshot_id),
        )
        .map_err(|e| {
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "token_sign_error"])
                .inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    state
        .metrics
        .auth_refresh_total
        .with_label_values(&["success", "ok"])
        .inc();

    // Emit identity_auth.session_refreshed
    #[derive(Serialize)]
    struct Data {
        session_id: String,
        previous_session_id: String,
        user_id: String,
        expires_at: String,
    }
    let env = EventEnvelope::new(
        tenant_id.to_string(),
        state.producer.clone(),
        "identity_auth.session_refreshed".to_string(),
        Data {
            session_id: rotated.session_id.to_string(),
            previous_session_id: old_session_id.to_string(),
            user_id: user_id.to_string(),
            expires_at: rotated.expires_at.to_rfc3339(),
        },
    )
    .with_schema_version("1.0.0".to_string())
    .with_trace_id(Some(trace_id))
    .with_actor(user_id, "User".to_string())
    .with_mutation_class(Some("user-data".to_string()));
    if state
        .events
        .publish(
            "identity_auth.session_refreshed",
            "identity_auth.session.refreshed.v1.json",
            &env,
        )
        .await
        .is_err()
    {
        state
            .metrics
            .auth_nats_publish_fail_total
            .with_label_values(&["identity_auth.session_refreshed"])
            .inc();
    }

    // Build response: access token in body, rotated refresh cookie in Set-Cookie.
    let max_age = (rotated.expires_at - Utc::now()).num_seconds().max(0);
    let cookie_value = cookies::build_set_cookie(&rotated.raw_token, max_age, state.cookie_secure);
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_str(&cookie_value).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok((
        StatusCode::OK,
        resp_headers,
        Json(AccessTokenResponse {
            token_type: "Bearer",
            access_token: access,
            expires_in_seconds: state.access_ttl_minutes * 60,
        }),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Legacy body-based refresh (pre-cookie consumers)
// ---------------------------------------------------------------------------

async fn refresh_via_body(
    state: Arc<AuthState>,
    extensions: Extensions,
    headers: &HeaderMap,
    req: RefreshReq,
) -> Result<axum::response::Response, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let old_hash = hash_refresh_token(&req.refresh_token);
    let hash_prefix: String = old_hash.chars().take(12).collect();

    let mut tx = state.db.begin().await.map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    // Look up by token_hash alone — the row's tenant_id is authoritative.
    // Any tenant_id supplied by the client is ignored (back-compat window).
    let row = sqlx::query(
        r#"
        SELECT id, tenant_id, user_id, expires_at, revoked_at
        FROM refresh_tokens
        WHERE token_hash = $1
        "#,
    )
    .bind(&old_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "invalid"])
                .inc();
            return Err(err(StatusCode::UNAUTHORIZED, "invalid refresh token"));
        }
    };

    let token_id: Uuid = row.get("id");
    let tenant_id: Uuid = row.get("tenant_id");
    let user_id: Uuid = row.get("user_id");
    let expires_at: chrono::DateTime<Utc> = row.get("expires_at");
    let revoked_at: Option<chrono::DateTime<Utc>> = row.get("revoked_at");

    // Emit a structured deprecation log when the client still sends tenant_id.
    // The field is accepted but ignored; the DB row is the sole authority.
    if req.tenant_id.is_some() {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        let client_ip = headers
            .get("x-forwarded-for")
            .or_else(|| headers.get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        tracing::info!(
            deprecated_field = "tenant_id",
            user_agent = %user_agent,
            client_ip = %client_ip,
            "refresh.deprecated_field: tenant_id in body is ignored; server row is authoritative"
        );
    }

    if revoked_at.is_some() {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "revoked"])
            .inc();
        state
            .metrics
            .auth_refresh_replay_total
            .with_label_values(&[&tenant_id.to_string()])
            .inc();
        tracing::warn!(
            tenant_id = %tenant_id,
            user_id = %user_id,
            trace_id = %trace_id,
            token_hash_prefix = %hash_prefix,
            "security.refresh_replay_detected"
        );
        return Err(err(StatusCode::UNAUTHORIZED, "refresh token revoked"));
    }

    if expires_at < Utc::now() {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "expired"])
            .inc();
        return Err(err(StatusCode::UNAUTHORIZED, "refresh token expired"));
    }

    // Rate limit using the authoritative tenant_id from the DB row.
    if let Err(wait) = state.keyed_limits.check_refresh(
        &tenant_id.to_string(),
        &hash_prefix,
        state.refresh_per_min_per_token,
    ) {
        let _ = tx.rollback().await;
        state
            .metrics
            .auth_rate_limited_total
            .with_label_values(&["refresh"])
            .inc();
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "rate_limited"])
            .inc();
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
    }

    if let Some(client) = &state.tenant_registry {
        match client.get_tenant_gate(tenant_id, &state.metrics).await {
            Ok(TenantGate::Allow) | Ok(TenantGate::DenyNewLogin { .. }) => {}
            Ok(TenantGate::Deny { status }) => {
                state
                    .metrics
                    .auth_tenant_status_denied_total
                    .with_label_values(&[&status])
                    .inc();
                let _ = tx.rollback().await;
                return Err(err(StatusCode::FORBIDDEN, "tenant account inactive"));
            }
            Err(_) => {
                state
                    .metrics
                    .auth_tenant_status_denied_total
                    .with_label_values(&["unavailable"])
                    .inc();
                let _ = tx.rollback().await;
                return Err(err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "tenant status service unavailable",
                ));
            }
        }
    }

    sqlx::query(
        r#"
        UPDATE refresh_tokens
        SET revoked_at = NOW(), last_used_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(token_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let new_raw = generate_refresh_token();
    let new_hash = hash_refresh_token(&new_raw);
    let new_expires_at = Utc::now() + Duration::days(state.refresh_ttl_days);

    let new_token_id: Uuid = sqlx::query(
        r#"
        INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(&new_hash)
    .bind(new_expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?
    .get("id");

    super::concurrency::rotate_lease_in_tx(&mut tx, token_id, new_token_id)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rotate lease: {e}"),
            )
        })?;

    tx.commit()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let roles = crate::db::rbac::list_roles_for_user(&state.db, tenant_id, user_id)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac roles: {e}"),
            )
        })?
        .into_iter()
        .map(|r| r.name)
        .collect::<Vec<_>>();
    let perms = crate::db::rbac::effective_permissions_for_user(&state.db, tenant_id, user_id)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac perms: {e}"),
            )
        })?;
    let role_snapshot_id = super::jwt::compute_role_snapshot_id(&roles);

    let access = state
        .jwt
        .sign_access_token_enriched(
            tenant_id,
            user_id,
            roles,
            perms,
            super::jwt::actor_type::USER,
            state.access_ttl_minutes,
            Some(new_token_id),
            Some(role_snapshot_id),
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    state
        .metrics
        .auth_refresh_total
        .with_label_values(&["success", "ok"])
        .inc();

    #[derive(Serialize)]
    struct Data {
        user_id: String,
    }
    let env = EventEnvelope::new(
        tenant_id.to_string(),
        state.producer.clone(),
        "auth.token_refreshed".to_string(),
        Data {
            user_id: user_id.to_string(),
        },
    )
    .with_schema_version("1.0.0".to_string())
    .with_trace_id(Some(trace_id))
    .with_actor(user_id, "User".to_string())
    .with_mutation_class(Some("user-data".to_string()));
    if state
        .events
        .publish("auth.token_refreshed", "auth.token.refreshed.v1.json", &env)
        .await
        .is_err()
    {
        state
            .metrics
            .auth_nats_publish_fail_total
            .with_label_values(&["auth.token_refreshed"])
            .inc();
    }

    Ok((
        StatusCode::OK,
        Json(TokenResponse {
            token_type: "Bearer",
            access_token: access,
            expires_in_seconds: state.access_ttl_minutes * 60,
            refresh_token: new_raw,
        }),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// POST /api/auth/logout — cookie-aware
// ---------------------------------------------------------------------------

#[utoipa::path(post, path = "/api/auth/logout", tag = "Auth",
    request_body = LogoutReq,
    responses(
        (status = 200, description = "Logged out", body = OkResponse),
        (status = 500, description = "Internal error"),
    ))]
pub async fn logout(
    State(state): State<Arc<AuthState>>,
    _extensions: Extensions,
    headers: HeaderMap,
    body: Option<Json<LogoutReq>>,
) -> Result<axum::response::Response, ApiErr> {
    // Revoke any session pointed to by a cookie, and simultaneously honour any
    // body-based legacy call. Emit events for whichever flow fired.
    let mut any_revoked = false;
    let mut cookie_session_info: Option<(Uuid, Uuid, Uuid)> = None; // (session_id, tenant, user)

    if let Some(raw) = cookies::read_refresh_cookie(&headers) {
        let hash = hash_refresh_token(&raw);
        if let Ok(Some(row)) = sqlx::query(
            "SELECT session_id, tenant_id, user_id FROM refresh_sessions WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(&hash)
        .fetch_optional(&state.db)
        .await
        {
            let session_id: Uuid = row.get("session_id");
            let tenant_id: Uuid = row.get("tenant_id");
            let user_id: Uuid = row.get("user_id");
            cookie_session_info = Some((session_id, tenant_id, user_id));
        }
        if let Ok(n) = refresh_sessions::revoke_by_token_hash(&state.db, &hash, "logout").await {
            if n > 0 {
                any_revoked = true;
            }
        }
    }

    if let Some(Json(req)) = body {
        let hash = hash_refresh_token(&req.refresh_token);
        let res = sqlx::query(
            r#"
            UPDATE refresh_tokens
            SET revoked_at = NOW(), last_used_at = NOW()
            WHERE tenant_id = $1 AND token_hash = $2 AND revoked_at IS NULL
            "#,
        )
        .bind(req.tenant_id)
        .bind(&hash)
        .execute(&state.db)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_logout_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
        })?;

        if res.rows_affected() > 0 {
            any_revoked = true;
            let _ = super::concurrency::revoke_lease_by_token_hash(&state.db, req.tenant_id, &hash)
                .await;
        }
    }

    if !any_revoked {
        state
            .metrics
            .auth_logout_total
            .with_label_values(&["failure", "invalid"])
            .inc();
        return Err(err(StatusCode::UNAUTHORIZED, "invalid refresh token"));
    }

    state
        .metrics
        .auth_logout_total
        .with_label_values(&["success", "ok"])
        .inc();

    // Emit revocation event for the cookie-flow session (best effort).
    if let Some((session_id, tenant_id, user_id)) = cookie_session_info {
        #[derive(Serialize)]
        struct Data {
            session_id: String,
            user_id: String,
            reason: String,
        }
        let env = EventEnvelope::new(
            tenant_id.to_string(),
            state.producer.clone(),
            "identity_auth.session_revoked".to_string(),
            Data {
                session_id: session_id.to_string(),
                user_id: user_id.to_string(),
                reason: "logout".to_string(),
            },
        )
        .with_schema_version("1.0.0".to_string())
        .with_trace_id(Some(Uuid::new_v4().to_string()))
        .with_actor(user_id, "User".to_string())
        .with_mutation_class(Some("user-data".to_string()));
        if state
            .events
            .publish(
                "identity_auth.session_revoked",
                "identity_auth.session.revoked.v1.json",
                &env,
            )
            .await
            .is_err()
        {
            state
                .metrics
                .auth_nats_publish_fail_total
                .with_label_values(&["identity_auth.session_revoked"])
                .inc();
        }
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_str(&cookies::build_clear_cookie(state.cookie_secure))
            .unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok((StatusCode::OK, resp_headers, Json(OkResponse { ok: true })).into_response())
}

// ---------------------------------------------------------------------------
// GET /api/auth/sessions — list active sessions for a user
// POST /api/auth/sessions/:id/revoke — revoke a specific session
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/auth/sessions", tag = "Auth",
    params(SessionsQuery),
    responses(
        (status = 200, description = "Active refresh sessions for the user", body = SessionsListResponse),
        (status = 500, description = "Internal error"),
    ),
    security(("bearer" = [])))]
pub async fn list_sessions(
    State(state): State<Arc<AuthState>>,
    Query(q): Query<SessionsQuery>,
) -> Result<impl IntoResponse, ApiErr> {
    let sessions = refresh_sessions::list_active_for_user(&state.db, q.tenant_id, q.user_id)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;
    Ok((StatusCode::OK, Json(SessionsListResponse { sessions })))
}

#[utoipa::path(post, path = "/api/auth/sessions/{session_id}/revoke", tag = "Auth",
    params(("session_id" = Uuid, Path, description = "Refresh session ID to revoke")),
    request_body = RevokeSessionReq,
    responses(
        (status = 200, description = "Session revoked", body = OkResponse),
        (status = 404, description = "Session not found or already revoked"),
        (status = 500, description = "Internal error"),
    ),
    security(("bearer" = [])))]
pub async fn revoke_session(
    State(state): State<Arc<AuthState>>,
    Path(session_id): Path<Uuid>,
    extensions: Extensions,
    Json(req): Json<RevokeSessionReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let affected = refresh_sessions::revoke_by_id(
        &state.db,
        session_id,
        req.tenant_id,
        req.user_id,
        "user_revoked",
    )
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    if affected == 0 {
        return Err(err(StatusCode::NOT_FOUND, "session not found"));
    }

    #[derive(Serialize)]
    struct Data {
        session_id: String,
        user_id: String,
        reason: String,
    }
    let env = EventEnvelope::new(
        req.tenant_id.to_string(),
        state.producer.clone(),
        "identity_auth.session_revoked".to_string(),
        Data {
            session_id: session_id.to_string(),
            user_id: req.user_id.to_string(),
            reason: "user_revoked".to_string(),
        },
    )
    .with_schema_version("1.0.0".to_string())
    .with_trace_id(Some(trace_id))
    .with_actor(req.user_id, "User".to_string())
    .with_mutation_class(Some("user-data".to_string()));
    if state
        .events
        .publish(
            "identity_auth.session_revoked",
            "identity_auth.session.revoked.v1.json",
            &env,
        )
        .await
        .is_err()
    {
        state
            .metrics
            .auth_nats_publish_fail_total
            .with_label_values(&["identity_auth.session_revoked"])
            .inc();
    }

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}
