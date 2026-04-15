use axum::{extract::State, Extension, Json};
use chrono::{Duration, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{auth::hash_refresh_token, db::portal_repo, outbox::enqueue_portal_event};

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
            let existing = portal_repo::find_idempotency(&state.pool, req.tenant_id, "login", key)
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

    let user =
        portal_repo::find_user_by_email(&state.pool, req.tenant_id, &req.email.to_lowercase())
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

    portal_repo::insert_refresh_token_tx(
        &mut tx,
        Uuid::new_v4(),
        user.tenant_id,
        user.id,
        &refresh_hash,
        refresh_expires_at,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal auth db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    portal_repo::update_last_login_tx(&mut tx, user.id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    if let Some(key) = req.idempotency_key.as_ref() {
        if !key.trim().is_empty() {
            portal_repo::insert_idempotency_tx(
                &mut tx,
                user.tenant_id,
                "login",
                key,
                &serde_json::json!({"access_token": access_token, "refresh_token": refresh_raw}),
            )
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

    let row = portal_repo::find_refresh_token(&state.pool, &old_hash)
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
    portal_repo::revoke_refresh_token_tx(&mut tx, &old_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal auth db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    portal_repo::insert_refresh_token_tx(
        &mut tx,
        Uuid::new_v4(),
        tenant_id,
        user_id,
        &new_refresh_hash,
        new_exp,
    )
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

    let row = portal_repo::find_active_refresh_token(&state.pool, &hash)
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

    portal_repo::revoke_refresh_token_tx(&mut tx, &hash)
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
