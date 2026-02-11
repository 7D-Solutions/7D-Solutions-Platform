use crate::{
    events::{envelope::EventEnvelope, publisher::EventPublisher},
    middleware::tracing::get_trace_id_from_extensions,
};
use axum::{
    extract::State,
    http::{Extensions, StatusCode},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

use super::{
    jwt::JwtKeys,
    password::{hash_password, verify_password, PasswordPolicy},
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
) -> Result<Json<OkResponse>, (StatusCode, String)> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let hash = hash_password(&state.pwd, &req.password)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let res = sqlx::query(
        r#"
        INSERT INTO credentials (tenant_id, user_id, email, password_hash)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.user_id)
    .bind(req.email.to_lowercase())
    .bind(hash)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => {
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
                data: Data { user_id: req.user_id.to_string(), email: req.email },
            };

            let _ = state.events.publish(
                "auth.events.user.registered",
                "auth.user.registered.v1.json",
                &env
            ).await;

            Ok(Json(OkResponse { ok: true }))
        }
        Err(e) => {
            if let Some(db_err) = e.as_database_error() {
                if db_err.code().as_deref() == Some("23505") {
                    return Err((StatusCode::CONFLICT, "credential already exists".into()));
                }
            }
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))
        }
    }
}

pub async fn login(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<LoginReq>,
) -> Result<Json<TokenResponse>, (StatusCode, String)> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let row = sqlx::query(
        r#"
        SELECT user_id, password_hash, is_active
        FROM credentials
        WHERE tenant_id = $1 AND email = $2
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.email.to_lowercase())
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let row = match row {
        Some(r) => r,
        None => return Err((StatusCode::UNAUTHORIZED, "invalid credentials".into())),
    };

    let user_id: Uuid = row.get("user_id");
    let password_hash: String = row.get("password_hash");
    let is_active: bool = row.get("is_active");

    if !is_active {
        return Err((StatusCode::FORBIDDEN, "account inactive".into()));
    }

    let ok = verify_password(&state.pwd, &req.password, &password_hash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if !ok {
        return Err((StatusCode::UNAUTHORIZED, "invalid credentials".into()));
    }

    let access = state
        .jwt
        .sign_access_token(req.tenant_id, user_id, state.access_ttl_minutes)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

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
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

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
    let _ = state.events.publish(
        "auth.events.user.logged_in",
        "auth.user.logged_in.v1.json",
        &env
    ).await;

    Ok(Json(TokenResponse {
        token_type: "Bearer",
        access_token: access,
        expires_in_seconds: state.access_ttl_minutes * 60,
        refresh_token: refresh_raw,
    }))
}

pub async fn refresh(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<RefreshReq>,
) -> Result<Json<TokenResponse>, (StatusCode, String)> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let old_hash = hash_refresh_token(&req.refresh_token);

    let mut tx = state.db.begin().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

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
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let row = match row {
        Some(r) => r,
        None => return Err((StatusCode::UNAUTHORIZED, "invalid refresh token".into())),
    };

    let token_id: Uuid = row.get("id");
    let user_id: Uuid = row.get("user_id");
    let expires_at: chrono::DateTime<Utc> = row.get("expires_at");
    let revoked_at: Option<chrono::DateTime<Utc>> = row.get("revoked_at");

    if revoked_at.is_some() || expires_at < Utc::now() {
        return Err((StatusCode::UNAUTHORIZED, "refresh token expired or revoked".into()));
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
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

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
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    tx.commit().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let access = state
        .jwt
        .sign_access_token(req.tenant_id, user_id, state.access_ttl_minutes)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

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
    let _ = state.events.publish(
        "auth.events.token.refreshed",
        "auth.token.refreshed.v1.json",
        &env
    ).await;

    Ok(Json(TokenResponse {
        token_type: "Bearer",
        access_token: access,
        expires_in_seconds: state.access_ttl_minutes * 60,
        refresh_token: new_raw,
    }))
}

pub async fn logout(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<LogoutReq>,
) -> Result<Json<OkResponse>, (StatusCode, String)> {
    let trace_id = get_trace_id_from_extensions(&extensions);

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
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    if res.rows_affected() == 0 {
        return Err((StatusCode::UNAUTHORIZED, "invalid refresh token".into()));
    }

    #[derive(Serialize)]
    struct Data {}
    let env = EventEnvelope {
        event_id: Uuid::new_v4(),
        event_type: "auth.user.logged_out".to_string(),
        schema_version: "auth.user.logged_out/v1".to_string(),
        occurred_at: Utc::now(),
        producer: state.producer.clone(),
        tenant_id: req.tenant_id,
        aggregate_type: "tenant".to_string(),
        aggregate_id: req.tenant_id,
        trace_id,
        causation_id: None,
        data: Data {},
    };
    let _ = state.events.publish(
        "auth.events.user.logged_out",
        "auth.user.logged_out.v1.json",
        &env
    ).await;

    Ok(Json(OkResponse { ok: true }))
}
