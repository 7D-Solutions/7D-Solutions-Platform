use crate::{
    auth::{
        handlers::{ApiErr, AuthState, err, err_retry_after},
        password_reset_repo::insert_reset_token,
        password_reset_tokens::{generate_raw_token, sha256_token_hash},
    },
    events::envelope::EventEnvelope,
    middleware::{client_ip::get_client_meta, tracing::get_trace_id_from_extensions},
};
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

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct GenericOkResponse {
    pub message: &'static str,
}

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
    if let Err(wait) = state.keyed_limits.check_forgot_email(&email, state.forgot_per_min_per_email) {
        return Err(err_retry_after(StatusCode::TOO_MANY_REQUESTS, wait, "rate limited"));
    }

    // Rate limit per-IP
    if let Err(wait) = state.keyed_limits.check_forgot_ip(&ip, state.forgot_per_min_per_ip) {
        return Err(err_retry_after(StatusCode::TOO_MANY_REQUESTS, wait, "rate limited"));
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
            return Ok((StatusCode::OK, Json(GenericOkResponse { message: FORGOT_PWD_MSG })));
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

        let env = EventEnvelope {
            event_id: Uuid::new_v4(),
            event_type: "auth.password_reset_requested".to_string(),
            schema_version: "auth.password_reset_requested/v1".to_string(),
            occurred_at: Utc::now(),
            producer: state.producer.clone(),
            tenant_id,
            aggregate_type: "user".to_string(),
            aggregate_id: user_id,
            trace_id: trace_id.clone(),
            causation_id: None,
            data: PasswordResetRequestedData {
                user_id: user_id.to_string(),
                email,
                raw_token,
                expires_at: expires_at.to_rfc3339(),
                correlation_id: trace_id,
            },
        };

        if let Err(_) = state
            .events
            .publish(
                "auth.events.password_reset_requested",
                "auth.password_reset_requested.v1.json",
                &env,
            )
            .await
        {
            tracing::warn!(
                user_id = %user_id,
                "auth.forgot_password.event_publish_failed"
            );
            // Still return 200 — event bus failure must not reveal user existence
        }
    }

    Ok((StatusCode::OK, Json(GenericOkResponse { message: FORGOT_PWD_MSG })))
}
