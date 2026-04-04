use crate::{
    auth::{
        handlers::{err, err_retry_after, ApiErr, AuthState, OkResponse},
        password::hash_password,
        password_policy::{validate_password, PasswordRules},
        password_reset_repo::{claim_reset_token, insert_reset_token},
        password_reset_tokens::{generate_raw_token, sha256_token_hash},
    },
    middleware::{client_ip::get_client_meta, tracing::get_trace_id_from_extensions},
};
use event_bus::EventEnvelope;
use axum::{
    extract::State,
    http::{Extensions, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

const FORGOT_PWD_MSG: &str = "If an account exists, a reset email has been sent.";

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GenericOkResponse {
    pub message: &'static str,
}

#[utoipa::path(post, path = "/api/auth/forgot-password", tag = "Password Reset",
    security(()),
    request_body = ForgotPasswordRequest,
    responses(
        (status = 200, description = "If account exists, reset email sent", body = GenericOkResponse),
        (status = 429, description = "Rate limited"),
    ))]
pub async fn forgot_password(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);
    let ip = get_client_meta(&extensions)
        .map(|m| m.ip)
        .unwrap_or_else(|| "unknown".to_string());

    let email = req.email.trim().to_lowercase();

    // Rate limit per-email BEFORE any DB work (prevent enumeration via timing)
    if let Err(wait) = state
        .keyed_limits
        .check_forgot_email(&email, state.forgot_per_min_per_email)
    {
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
    }

    // Rate limit per-IP
    if let Err(wait) = state
        .keyed_limits
        .check_forgot_ip(&ip, state.forgot_per_min_per_ip)
    {
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
    }

    // Look up user by email — LIMIT 1 (email unique per tenant; v1 picks first match)
    let row = sqlx::query(
        r#"
        SELECT user_id, tenant_id FROM credentials WHERE email = $1 LIMIT 1
        "#,
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    // Always return 200 — never reveal whether the email exists
    if let Some(row) = row {
        let user_id: Uuid = row.get("user_id");
        let tenant_id: Uuid = row.get("tenant_id");

        let raw_token = generate_raw_token();
        let token_hash = sha256_token_hash(&raw_token);
        let expires_at = Utc::now() + chrono::Duration::minutes(state.password_reset_ttl_minutes);

        if let Err(e) = insert_reset_token(&state.db, user_id, &token_hash, expires_at).await {
            tracing::error!(
                user_id = %user_id,
                trace_id = %trace_id,
                err = %e,
                "auth.forgot_password.insert_token_error"
            );
            // Still return 200 — do not reveal server-side errors to caller
            return Ok((
                StatusCode::OK,
                Json(GenericOkResponse {
                    message: FORGOT_PWD_MSG,
                }),
            ));
        }

        #[derive(Serialize)]
        struct PasswordResetRequestedData {
            user_id: String,
            email: String,
            // WARNING: raw_token is sensitive — must not appear in logs;
            // this event requires TLS transport on the NATS bus.
            raw_token: String,
            expires_at: String,
            correlation_id: String,
        }

        let env = EventEnvelope::new(
            tenant_id.to_string(),
            state.producer.clone(),
            "auth.password_reset_requested".to_string(),
            PasswordResetRequestedData {
                user_id: user_id.to_string(),
                email,
                raw_token,
                expires_at: expires_at.to_rfc3339(),
                correlation_id: trace_id.clone(),
            },
        )
        .with_schema_version("1.0.0".to_string())
        .with_trace_id(Some(trace_id))
        .with_mutation_class(Some("user-data".to_string()));

        if state
            .events
            .publish(
                "auth.password_reset_requested",
                "auth.password_reset_requested.v1.json",
                &env,
            )
            .await
            .is_err()
        {
            tracing::warn!(
                user_id = %user_id,
                "auth.forgot_password.event_publish_failed"
            );
            // Still return 200 — event bus failure must not reveal user existence
        }
    }

    Ok((
        StatusCode::OK,
        Json(GenericOkResponse {
            message: FORGOT_PWD_MSG,
        }),
    ))
}

// ---------------------------------------------------------------------------
// POST /api/auth/reset-password
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

#[utoipa::path(post, path = "/api/auth/reset-password", tag = "Password Reset",
    security(()),
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "Password reset successful", body = crate::auth::handlers::OkResponse),
        (status = 400, description = "Invalid/expired token or weak password"),
        (status = 429, description = "Rate limited"),
    ))]
pub async fn reset_password(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);
    let ip = get_client_meta(&extensions)
        .map(|m| m.ip)
        .unwrap_or_else(|| "unknown".to_string());

    // 1. Rate limit per-IP BEFORE any DB work
    if let Err(wait) = state
        .keyed_limits
        .check_reset_ip(&ip, state.reset_per_min_per_ip)
    {
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
    }

    // 2. Validate new_password BEFORE touching DB
    let rules = PasswordRules::default();
    if let Err(e) = validate_password(&rules, &req.new_password) {
        return Err(err(StatusCode::BAD_REQUEST, e.to_string()));
    }

    // 3. Compute SHA-256 hash and atomically claim the reset token
    let token_hash = sha256_token_hash(&req.token);
    let user_id = match claim_reset_token(&state.db, &token_hash).await {
        Ok(Some(uid)) => uid,
        Ok(None) => return Err(err(StatusCode::BAD_REQUEST, "invalid or expired token")),
        Err(e) => {
            return Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("db error: {e}"),
            ))
        }
    };

    // 4. Hash new password with argon2id
    let new_hash = hash_password(&state.pwd, &req.new_password)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    // 5. Update password in credentials
    sqlx::query(
        r#"UPDATE credentials SET password_hash = $1, updated_at = NOW() WHERE user_id = $2"#,
    )
    .bind(&new_hash)
    .bind(user_id)
    .execute(&state.db)
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("password update: {e}"),
        )
    })?;

    // 6. Hard-delete all session leases for this user (any error → 500)
    sqlx::query("DELETE FROM session_leases WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session revocation: {e}"),
            )
        })?;

    // 7. Hard-delete all refresh tokens for this user (any error → 500)
    sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("token revocation: {e}"),
            )
        })?;

    // 8. Publish completion event
    //    Resolve tenant_id from credentials for the event envelope (LIMIT 1 — v1 acceptable).
    let tenant_id = sqlx::query("SELECT tenant_id FROM credentials WHERE user_id = $1 LIMIT 1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .map(|r| r.get::<Uuid, _>("tenant_id"))
        .unwrap_or_else(Uuid::nil);

    #[derive(Serialize)]
    struct PasswordResetCompletedData {
        user_id: String,
        correlation_id: String,
    }

    let env = EventEnvelope::new(
        tenant_id.to_string(),
        state.producer.clone(),
        "auth.password_reset_completed".to_string(),
        PasswordResetCompletedData {
            user_id: user_id.to_string(),
            correlation_id: trace_id.clone(),
        },
    )
    .with_schema_version("1.0.0".to_string())
    .with_trace_id(Some(trace_id))
    .with_mutation_class(Some("user-data".to_string()));

    if state
        .events
        .publish(
            "auth.password_reset_completed",
            "auth.password_reset_completed.v1.json",
            &env,
        )
        .await
        .is_err()
    {
        tracing::warn!(user_id = %user_id, "auth.reset_password.event_publish_failed");
        // Event failure is logged but does not fail the response — password is already reset.
    }

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}
