use crate::{
    events::envelope::EventEnvelope,
    middleware::tracing::get_trace_id_from_extensions,
};
use axum::{
    extract::State,
    http::{Extensions, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use super::handlers::{err, err_retry_after, ApiErr, AuthState, OkResponse, TokenResponse};
use super::refresh::{generate_refresh_token, hash_refresh_token};

#[derive(Debug, Deserialize)]
pub struct RefreshReq {
    pub tenant_id: Uuid,
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutReq {
    pub tenant_id: Uuid,
    pub refresh_token: String,
}

pub async fn refresh(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<RefreshReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let old_hash = hash_refresh_token(&req.refresh_token);
    let hash_prefix: String = old_hash.chars().take(12).collect();

    if let Err(wait) = state.keyed_limits.check_refresh(
        &req.tenant_id.to_string(),
        &hash_prefix,
        state.refresh_per_min_per_token,
    ) {
        state.metrics.auth_rate_limited_total.with_label_values(&["refresh"]).inc();
        state.metrics.auth_refresh_total.with_label_values(&["failure", "rate_limited"]).inc();
        return Err(err_retry_after(StatusCode::TOO_MANY_REQUESTS, wait, "rate limited"));
    }

    let mut tx = state.db.begin().await.map_err(|e| {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let row = sqlx::query(
        r#"
        SELECT id, user_id, expires_at, revoked_at
        FROM refresh_tokens
        WHERE tenant_id = $1 AND token_hash = $2
        "#,
    )
    .bind(req.tenant_id)
    .bind(&old_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            state.metrics.auth_refresh_total.with_label_values(&["failure", "invalid"]).inc();
            return Err(err(StatusCode::UNAUTHORIZED, "invalid refresh token"));
        }
    };

    let token_id: Uuid = row.get("id");
    let user_id: Uuid = row.get("user_id");
    let expires_at: chrono::DateTime<Utc> = row.get("expires_at");
    let revoked_at: Option<chrono::DateTime<Utc>> = row.get("revoked_at");

    if revoked_at.is_some() {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "revoked"]).inc();
        state.metrics.auth_refresh_replay_total.with_label_values(&[&req.tenant_id.to_string()]).inc();

        let client = crate::middleware::client_ip::get_client_meta(&extensions);
        let ip = client.as_ref().map(|c| c.ip.as_str()).unwrap_or("unknown");
        let ua = client.as_ref().and_then(|c| c.user_agent.as_deref()).unwrap_or("unknown");

        tracing::warn!(
            tenant_id = %req.tenant_id,
            user_id = %user_id,
            trace_id = %trace_id,
            token_hash_prefix = %hash_prefix,
            client_ip = %ip,
            user_agent = %ua,
            "security.refresh_replay_detected"
        );
        return Err(err(StatusCode::UNAUTHORIZED, "refresh token revoked"));
    }

    if expires_at < Utc::now() {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "expired"]).inc();
        return Err(err(StatusCode::UNAUTHORIZED, "refresh token expired"));
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
    .map_err(|e| {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let new_raw = generate_refresh_token();
    let new_hash = hash_refresh_token(&new_raw);
    let new_expires_at = Utc::now() + Duration::days(state.refresh_ttl_days);

    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(req.tenant_id)
    .bind(user_id)
    .bind(&new_hash)
    .bind(new_expires_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    tx.commit().await.map_err(|e| {
        state.metrics.auth_refresh_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let access = state.jwt
        .sign_access_token(req.tenant_id, user_id, state.access_ttl_minutes)
        .map_err(|e| {
            state.metrics.auth_refresh_total.with_label_values(&["failure", "token_sign_error"]).inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    state.metrics.auth_refresh_total.with_label_values(&["success", "ok"]).inc();

    #[derive(Serialize)]
    struct Data { user_id: String }
    let env = EventEnvelope {
        event_id: Uuid::new_v4(),
        event_type: "auth.token.refreshed".to_string(),
        schema_version: "auth.token.refreshed/v1".to_string(),
        occurred_at: Utc::now(),
        producer: state.producer.clone(),
        tenant_id: req.tenant_id,
        aggregate_type: "user".to_string(),
        aggregate_id: user_id,
        trace_id,
        causation_id: None,
        data: Data { user_id: user_id.to_string() },
    };

    if let Err(_) = state.events.publish(
        "auth.events.token.refreshed",
        "auth.token.refreshed.v1.json",
        &env
    ).await {
        state.metrics.auth_nats_publish_fail_total.with_label_values(&["auth.token.refreshed"]).inc();
    }

    Ok((
        StatusCode::OK,
        Json(TokenResponse {
            token_type: "Bearer",
            access_token: access,
            expires_in_seconds: state.access_ttl_minutes * 60,
            refresh_token: new_raw,
        }),
    ))
}

pub async fn logout(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<LogoutReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let _trace_id = get_trace_id_from_extensions(&extensions);

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
        state.metrics.auth_logout_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    if res.rows_affected() == 0 {
        state.metrics.auth_logout_total.with_label_values(&["failure", "invalid"]).inc();
        return Err(err(StatusCode::UNAUTHORIZED, "invalid refresh token"));
    }

    state.metrics.auth_logout_total.with_label_values(&["success", "ok"]).inc();

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}
