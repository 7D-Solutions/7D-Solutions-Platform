use crate::{
    events::{envelope::EventEnvelope, publisher::EventPublisher},
    metrics::Metrics,
    middleware::tracing::get_trace_id_from_extensions,
    rate_limit::KeyedLimiters,
};
use axum::{
    extract::State,
    http::{Extensions, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

use super::{
    concurrency::HashConcurrencyLimiter,
    jwt::JwtKeys,
    password::{hash_password, verify_password, PasswordPolicy},
    password_policy::{validate_password, PasswordRules},
    refresh::{generate_refresh_token, hash_refresh_token},
};

#[derive(Clone)]
pub struct AuthState {
    pub db: PgPool,
    pub jwt: JwtKeys,
    pub pwd: PasswordPolicy,
    pub access_ttl_minutes: i64,
    pub refresh_ttl_days: i64,
    pub events: EventPublisher,
    pub producer: String,

    pub metrics: Metrics,
    pub keyed_limits: KeyedLimiters,
    pub hash_limiter: HashConcurrencyLimiter,

    // Configurable policies
    pub lockout_threshold: i32,
    pub lockout_minutes: i64,

    pub login_per_min_per_email: u32,
    pub register_per_min_per_email: u32,
    pub refresh_per_min_per_token: u32,
}

type ApiErr = (StatusCode, HeaderMap, String);

fn err(code: StatusCode, msg: impl Into<String>) -> ApiErr {
    (code, HeaderMap::new(), msg.into())
}

fn err_retry_after(code: StatusCode, wait: std::time::Duration, msg: impl Into<String>) -> ApiErr {
    let mut headers = HeaderMap::new();
    let secs = wait.as_secs().max(1).to_string();
    headers.insert("Retry-After", HeaderValue::from_str(&secs).unwrap_or(HeaderValue::from_static("1")));
    (code, headers, msg.into())
}

#[derive(Debug, Deserialize)]
pub struct RegisterReq {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginReq {
    pub tenant_id: Uuid,
    pub email: String,
    pub password: String,
}

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

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token_type: &'static str,
    pub access_token: String,
    pub expires_in_seconds: i64,
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

pub async fn register(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<RegisterReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        state.metrics.auth_register_total.with_label_values(&["failure", "invalid_email"]).inc();
        return Err(err(StatusCode::BAD_REQUEST, "invalid email"));
    }

    // Password policy validation FIRST (before rate limit, before hash)
    let rules = PasswordRules::default();
    if let Err(e) = validate_password(&rules, &req.password) {
        state.metrics.auth_register_total.with_label_values(&["failure", "weak_password"]).inc();
        return Err(err(StatusCode::BAD_REQUEST, e.to_string()));
    }

    // keyed limit BEFORE hashing
    if let Err(wait) = state.keyed_limits.check_register_email(
        &req.tenant_id.to_string(),
        &email,
        state.register_per_min_per_email,
    ) {
        state.metrics.auth_rate_limited_total.with_label_values(&["email"]).inc();
        state.metrics.auth_register_total.with_label_values(&["failure", "rate_limited"]).inc();
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
    }

    // Acquire semaphore permit for hashing
    let _permit = match state.hash_limiter.acquire().await {
        Ok(p) => p,
        Err(_) => {
            state.metrics.auth_register_total.with_label_values(&["failure", "hash_busy"]).inc();
            tracing::warn!(tenant_id=%req.tenant_id, trace_id=%trace_id, "auth.hash_busy");
            return Err(err(StatusCode::SERVICE_UNAVAILABLE, "auth busy"));
        }
    };

    let hash = hash_password(&state.pwd, &req.password)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    let res = sqlx::query(
        r#"
        INSERT INTO credentials (tenant_id, user_id, email, password_hash)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.user_id)
    .bind(email.clone())
    .bind(hash)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => {
            state.metrics.auth_register_total.with_label_values(&["success", "ok"]).inc();

            #[derive(Serialize)]
            struct Data { user_id: String, email: String }

            let env = EventEnvelope {
                event_id: Uuid::new_v4(),
                event_type: "auth.user.registered".to_string(),
                schema_version: "auth.user.registered/v1".to_string(),
                occurred_at: Utc::now(),
                producer: state.producer.clone(),
                tenant_id: req.tenant_id,
                aggregate_type: "user".to_string(),
                aggregate_id: req.user_id,
                trace_id,
                causation_id: None,
                data: Data { user_id: req.user_id.to_string(), email: email.clone() },
            };

            if let Err(_) = state.events.publish(
                "auth.events.user.registered",
                "auth.user.registered.v1.json",
                &env
            ).await {
                state.metrics.auth_nats_publish_fail_total.with_label_values(&["auth.user.registered"]).inc();
            }

            Ok((StatusCode::OK, Json(OkResponse { ok: true })))
        }
        Err(e) => {
            state.metrics.auth_register_total.with_label_values(&["failure", "db_error"]).inc();
            if let Some(db_err) = e.as_database_error() {
                if db_err.code().as_deref() == Some("23505") {
                    return Err(err(StatusCode::CONFLICT, "credential already exists"));
                }
            }
            Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))
        }
    }
}

pub async fn login(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<LoginReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        state.metrics.auth_login_total.with_label_values(&["failure", "invalid_email"]).inc();
        return Err(err(StatusCode::BAD_REQUEST, "invalid email"));
    }

    // keyed limit BEFORE hashing
    if let Err(wait) = state.keyed_limits.check_login_email(
        &req.tenant_id.to_string(),
        &email,
        state.login_per_min_per_email,
    ) {
        state.metrics.auth_rate_limited_total.with_label_values(&["email"]).inc();
        state.metrics.auth_login_total.with_label_values(&["failure", "rate_limited"]).inc();
        return Err(err_retry_after(StatusCode::TOO_MANY_REQUESTS, wait, "rate limited"));
    }

    let row = sqlx::query(
        r#"
        SELECT user_id, password_hash, is_active, failed_login_count, lock_until
        FROM credentials
        WHERE tenant_id = $1 AND email = $2
        "#,
    )
    .bind(req.tenant_id)
    .bind(email.clone())
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        state.metrics.auth_login_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            state.metrics.auth_login_total.with_label_values(&["failure", "not_found"]).inc();
            return Err(err(StatusCode::UNAUTHORIZED, "invalid credentials"));
        }
    };

    let user_id: Uuid = row.get("user_id");
    let password_hash: String = row.get("password_hash");
    let is_active: bool = row.get("is_active");
    let lock_until: Option<chrono::DateTime<Utc>> = row.get("lock_until");

    if !is_active {
        state.metrics.auth_login_total.with_label_values(&["failure", "inactive"]).inc();
        return Err(err(StatusCode::FORBIDDEN, "account inactive"));
    }

    if let Some(lu) = lock_until {
        if lu > Utc::now() {
            state.metrics.auth_login_total.with_label_values(&["failure", "locked"]).inc();
            return Err(err(StatusCode::LOCKED, "account temporarily locked"));
        }
    }

    // Acquire semaphore permit for password verification
    let _permit = match state.hash_limiter.acquire().await {
        Ok(p) => p,
        Err(_) => {
            state.metrics.auth_login_total.with_label_values(&["failure", "hash_busy"]).inc();
            tracing::warn!(tenant_id=%req.tenant_id, email=%email, trace_id=%trace_id, "auth.hash_busy");
            return Err(err(StatusCode::SERVICE_UNAVAILABLE, "auth busy"));
        }
    };

    let t = Metrics::timer();
    let ok = verify_password(&state.pwd, &req.password, &password_hash)
        .map_err(|e| {
            state.metrics.auth_password_verify_duration_seconds.with_label_values(&["error"])
                .observe(t.elapsed().as_secs_f64());
            state.metrics.auth_login_total.with_label_values(&["failure", "verify_error"]).inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    if !ok {
        state.metrics.auth_password_verify_duration_seconds.with_label_values(&["fail"])
            .observe(t.elapsed().as_secs_f64());
        state.metrics.auth_login_total.with_label_values(&["failure", "invalid_password"]).inc();

        let _ = sqlx::query(
            r#"
            UPDATE credentials
            SET
              failed_login_count = failed_login_count + 1,
              last_failed_login_at = NOW(),
              lock_until = CASE
                WHEN (failed_login_count + 1) >= $3 THEN NOW() + ($4 || ' minutes')::interval
                ELSE lock_until
              END,
              updated_at = NOW()
            WHERE tenant_id = $1 AND email = $2
            "#,
        )
        .bind(req.tenant_id)
        .bind(email.clone())
        .bind(state.lockout_threshold)
        .bind(state.lockout_minutes)
        .execute(&state.db)
        .await;

        return Err(err(StatusCode::UNAUTHORIZED, "invalid credentials"));
    }

    state.metrics.auth_password_verify_duration_seconds.with_label_values(&["ok"])
        .observe(t.elapsed().as_secs_f64());

    let _ = sqlx::query(
        r#"
        UPDATE credentials
        SET failed_login_count = 0,
            lock_until = NULL,
            updated_at = NOW()
        WHERE tenant_id = $1 AND email = $2
        "#,
    )
    .bind(req.tenant_id)
    .bind(email.clone())
    .execute(&state.db)
    .await;

    let access = state.jwt
        .sign_access_token(req.tenant_id, user_id, state.access_ttl_minutes)
        .map_err(|e| {
            state.metrics.auth_login_total.with_label_values(&["failure", "token_sign_error"]).inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    let refresh_raw = generate_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_raw);
    let expires_at = Utc::now() + Duration::days(state.refresh_ttl_days);

    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(req.tenant_id)
    .bind(user_id)
    .bind(refresh_hash)
    .bind(expires_at)
    .execute(&state.db)
    .await
    .map_err(|e| {
        state.metrics.auth_login_total.with_label_values(&["failure", "db_error"]).inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    state.metrics.auth_login_total.with_label_values(&["success", "ok"]).inc();

    #[derive(Serialize)]
    struct Data { user_id: String }
    let env = EventEnvelope {
        event_id: Uuid::new_v4(),
        event_type: "auth.user.logged_in".to_string(),
        schema_version: "auth.user.logged_in/v1".to_string(),
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
        "auth.events.user.logged_in",
        "auth.user.logged_in.v1.json",
        &env
    ).await {
        state.metrics.auth_nats_publish_fail_total.with_label_values(&["auth.user.logged_in"]).inc();
    }

    Ok((
        StatusCode::OK,
        Json(TokenResponse {
            token_type: "Bearer",
            access_token: access,
            expires_in_seconds: state.access_ttl_minutes * 60,
            refresh_token: refresh_raw,
        }),
    ))
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
