use axum::{extract::State, Extension, Json};
use chrono::{Duration, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{auth::hash_refresh_token, outbox::enqueue_portal_event};

#[derive(sqlx::FromRow)]
struct PortalUserRow {
    id: Uuid,
    tenant_id: Uuid,
    party_id: Uuid,
    password_hash: String,
    is_active: bool,
    lock_until: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub tenant_id: Uuid,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
}

#[utoipa::path(
    post, path = "/portal/auth/login", tag = "Auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, body = ApiError), (status = 403, body = ApiError),
        (status = 423, description = "Account locked", body = ApiError),
    ),
)]
pub async fn login(
    State(state): State<Arc<crate::AppState>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if req.email.trim().is_empty() || req.password.is_empty() {
        return Err(with_request_id(
            ApiError::unauthorized("invalid_credentials"),
            &ctx,
        ));
    }

    if let Some(key) = req.idempotency_key.as_ref() {
        if !key.trim().is_empty() {
            let existing = sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT response FROM portal_idempotency WHERE tenant_id=$1 AND operation='login' AND idempotency_key=$2",
            )
            .bind(req.tenant_id)
            .bind(key)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "portal auth db error");
                with_request_id(ApiError::internal("Database error"), &ctx)
            })?;

            if let Some(response) = existing {
                let access_token = response
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let refresh_token = response
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !access_token.is_empty() && !refresh_token.is_empty() {
                    return Ok(Json(AuthResponse {
                        access_token,
                        refresh_token,
                        token_type: "Bearer".to_string(),
                    }));
                }
            }
        }
    }

    let user = sqlx::query_as::<_, PortalUserRow>(
        "SELECT id, tenant_id, party_id, password_hash, is_active, lock_until \
         FROM portal_users WHERE tenant_id=$1 AND email=$2",
    )
    .bind(req.tenant_id)
    .bind(req.email.to_lowercase())
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?
    .ok_or_else(|| with_request_id(ApiError::unauthorized("invalid_credentials"), &ctx))?;

    if !user.is_active {
        return Err(with_request_id(
            ApiError::forbidden("account_disabled"),
            &ctx,
        ));
    }

    if user.lock_until.is_some_and(|until| until > Utc::now()) {
        return Err(with_request_id(
            ApiError::new(423, "account_locked", "Account is temporarily locked"),
            &ctx,
        ));
    }

    if !crate::verify_password(&user.password_hash, &req.password) {
        return Err(with_request_id(
            ApiError::unauthorized("invalid_credentials"),
            &ctx,
        ));
    }

    let access_token = state
        .portal_jwt
        .issue_access_token(
            user.id,
            user.tenant_id,
            user.party_id,
            vec![platform_contracts::portal_identity::scopes::DOCUMENTS_READ.to_string()],
            state.config.access_token_ttl_minutes,
        )
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth error");
            with_request_id(ApiError::internal("Token generation failed"), &ctx)
        })?;

    let refresh_raw = crate::auth::generate_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_raw);
    let refresh_expires_at = Utc::now() + Duration::days(state.config.refresh_token_ttl_days);

    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    sqlx::query(
        "INSERT INTO portal_refresh_tokens (id, tenant_id, user_id, token_hash, expires_at) \
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(user.tenant_id)
    .bind(user.id)
    .bind(&refresh_hash)
    .bind(refresh_expires_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    sqlx::query("UPDATE portal_users SET last_login_at = NOW() WHERE id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    if let Some(key) = req.idempotency_key.as_ref() {
        if !key.trim().is_empty() {
            sqlx::query(
                "INSERT INTO portal_idempotency (tenant_id, operation, idempotency_key, response) VALUES ($1,'login',$2,$3) \
                 ON CONFLICT (tenant_id, operation, idempotency_key) DO NOTHING",
            )
            .bind(user.tenant_id)
            .bind(key)
            .bind(serde_json::json!({
                "access_token": access_token,
                "refresh_token": refresh_raw,
            }))
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "portal auth db error");
                with_request_id(ApiError::internal("Database error"), &ctx)
            })?;
        }
    }

    enqueue_portal_event(
        &mut tx,
        user.tenant_id,
        Some(user.id),
        platform_contracts::portal_identity::events::USER_LOGIN,
        serde_json::json!({"user_id": user.id, "tenant_id": user.tenant_id, "party_id": user.party_id}),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(Json(AuthResponse {
        access_token,
        refresh_token: refresh_raw,
        token_type: "Bearer".to_string(),
    }))
}

#[utoipa::path(
    post, path = "/portal/auth/refresh", tag = "Auth",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Token refreshed", body = AuthResponse),
        (status = 401, body = ApiError),
    ),
)]
pub async fn refresh(
    State(state): State<Arc<crate::AppState>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let old_hash = hash_refresh_token(&req.refresh_token);

    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            chrono::DateTime<Utc>,
            Option<chrono::DateTime<Utc>>,
        ),
    >(
        "SELECT rt.user_id, rt.tenant_id, u.party_id, rt.expires_at, rt.revoked_at \
         FROM portal_refresh_tokens rt \
         JOIN portal_users u ON u.id = rt.user_id \
         WHERE rt.token_hash = $1",
    )
    .bind(&old_hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?
    .ok_or_else(|| with_request_id(ApiError::unauthorized("invalid_credentials"), &ctx))?;

    let (user_id, tenant_id, party_id, expires_at, revoked_at) = row;
    if revoked_at.is_some() || expires_at < Utc::now() {
        return Err(with_request_id(
            ApiError::unauthorized("invalid_credentials"),
            &ctx,
        ));
    }

    let new_access = state
        .portal_jwt
        .issue_access_token(
            user_id,
            tenant_id,
            party_id,
            vec![platform_contracts::portal_identity::scopes::DOCUMENTS_READ.to_string()],
            state.config.access_token_ttl_minutes,
        )
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth error");
            with_request_id(ApiError::internal("Token generation failed"), &ctx)
        })?;

    let new_refresh = crate::auth::generate_refresh_token();
    let new_refresh_hash = hash_refresh_token(&new_refresh);
    let new_exp = Utc::now() + Duration::days(state.config.refresh_token_ttl_days);

    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;
    sqlx::query("UPDATE portal_refresh_tokens SET revoked_at = NOW() WHERE token_hash = $1")
        .bind(&old_hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    sqlx::query(
        "INSERT INTO portal_refresh_tokens (id, tenant_id, user_id, token_hash, expires_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(user_id)
    .bind(new_refresh_hash)
    .bind(new_exp)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    enqueue_portal_event(
        &mut tx,
        tenant_id,
        Some(user_id),
        platform_contracts::portal_identity::events::TOKEN_REFRESHED,
        serde_json::json!({"user_id": user_id, "tenant_id": tenant_id}),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(Json(AuthResponse {
        access_token: new_access,
        refresh_token: new_refresh,
        token_type: "Bearer".to_string(),
    }))
}

#[utoipa::path(
    post, path = "/portal/auth/logout", tag = "Auth",
    request_body = LogoutRequest,
    responses(
        (status = 200, description = "Logged out"),
        (status = 401, body = ApiError),
    ),
)]
pub async fn logout(
    State(state): State<Arc<crate::AppState>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<LogoutRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let hash = hash_refresh_token(&req.refresh_token);

    let row = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT user_id, tenant_id FROM portal_refresh_tokens WHERE token_hash=$1 AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?
    .ok_or_else(|| with_request_id(ApiError::unauthorized("invalid_credentials"), &ctx))?;

    let (user_id, tenant_id) = row;
    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    sqlx::query("UPDATE portal_refresh_tokens SET revoked_at=NOW() WHERE token_hash=$1")
        .bind(hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    enqueue_portal_event(
        &mut tx,
        tenant_id,
        Some(user_id),
        platform_contracts::portal_identity::events::USER_LOGOUT,
        serde_json::json!({"user_id": user_id, "tenant_id": tenant_id}),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(Json(serde_json::json!({"status": "ok"})))
}
