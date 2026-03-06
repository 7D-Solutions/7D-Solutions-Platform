use crate::{
    clients::tenant_registry::TenantGate,
    middleware::tracing::get_trace_id_from_extensions,
};
use event_bus::EventEnvelope;
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

    let mut tx = state.db.begin().await.map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
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
    let user_id: Uuid = row.get("user_id");
    let expires_at: chrono::DateTime<Utc> = row.get("expires_at");
    let revoked_at: Option<chrono::DateTime<Utc>> = row.get("revoked_at");

    if revoked_at.is_some() {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "revoked"])
            .inc();
        state
            .metrics
            .auth_refresh_replay_total
            .with_label_values(&[&req.tenant_id.to_string()])
            .inc();

        let client = crate::middleware::client_ip::get_client_meta(&extensions);
        let ip = client.as_ref().map(|c| c.ip.as_str()).unwrap_or("unknown");
        let ua = client
            .as_ref()
            .and_then(|c| c.user_agent.as_deref())
            .unwrap_or("unknown");

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
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "expired"])
            .inc();
        return Err(err(StatusCode::UNAUTHORIZED, "refresh token expired"));
    }

    // Tenant lifecycle gate: deny refresh for suspended/deleted tenants.
    // past_due tenants may still refresh (grace period — deny new logins only).
    if let Some(client) = &state.tenant_registry {
        match client.get_tenant_gate(req.tenant_id, &state.metrics).await {
            Ok(TenantGate::Allow) | Ok(TenantGate::DenyNewLogin { .. }) => {
                // Allow: active/trial/past_due may refresh
            }
            Ok(TenantGate::Deny { status }) => {
                state
                    .metrics
                    .auth_tenant_status_denied_total
                    .with_label_values(&[&status])
                    .inc();
                tracing::warn!(
                    tenant_id = %req.tenant_id,
                    user_id = %user_id,
                    status = %status,
                    trace_id = %trace_id,
                    "auth.tenant_status_denied_refresh"
                );
                let _ = tx.rollback().await;
                return Err(err(StatusCode::FORBIDDEN, "tenant account inactive"));
            }
            Err(_) => {
                state
                    .metrics
                    .auth_tenant_status_denied_total
                    .with_label_values(&["unavailable"])
                    .inc();
                tracing::warn!(
                    tenant_id = %req.tenant_id,
                    user_id = %user_id,
                    trace_id = %trace_id,
                    "auth.tenant_status_unavailable_deny_refresh"
                );
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
    .map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

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
    .bind(req.tenant_id)
    .bind(user_id)
    .bind(&new_hash)
    .bind(new_expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?
    .get("id");

    // Rotate the session lease: point to new token and refresh last_seen_at.
    super::concurrency::rotate_lease_in_tx(&mut tx, token_id, new_token_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rotate lease: {e}"),
            )
        })?;

    tx.commit().await.map_err(|e| {
        state
            .metrics
            .auth_refresh_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    // Resolve current RBAC roles + permissions for the refreshed token
    let roles = crate::db::rbac::list_roles_for_user(&state.db, req.tenant_id, user_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac roles: {e}"),
            )
        })?
        .into_iter()
        .map(|r| r.name)
        .collect::<Vec<_>>();

    let perms = crate::db::rbac::effective_permissions_for_user(&state.db, req.tenant_id, user_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_refresh_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac perms: {e}"),
            )
        })?;

    let role_snapshot_id = super::jwt::compute_role_snapshot_id(&roles);

    let access = state
        .jwt
        .sign_access_token_enriched(
            req.tenant_id,
            user_id,
            roles,
            perms,
            super::jwt::actor_type::USER,
            state.access_ttl_minutes,
            Some(new_token_id),
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

    #[derive(Serialize)]
    struct Data {
        user_id: String,
    }
    let env = EventEnvelope::new(
        req.tenant_id.to_string(),
        state.producer.clone(),
        "auth.token.refreshed".to_string(),
        Data {
            user_id: user_id.to_string(),
        },
    )
    .with_schema_version("auth.token.refreshed/v1".to_string())
    .with_trace_id(Some(trace_id))
    .with_actor(user_id, "User".to_string())
    .with_mutation_class(Some("user-data".to_string()));

    if state
        .events
        .publish(
            "auth.events.token.refreshed",
            "auth.token.refreshed.v1.json",
            &env,
        )
        .await
        .is_err()
    {
        state
            .metrics
            .auth_nats_publish_fail_total
            .with_label_values(&["auth.token.refreshed"])
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
        state
            .metrics
            .auth_logout_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    if res.rows_affected() == 0 {
        state
            .metrics
            .auth_logout_total
            .with_label_values(&["failure", "invalid"])
            .inc();
        return Err(err(StatusCode::UNAUTHORIZED, "invalid refresh token"));
    }

    // Revoke the session lease (best-effort — token already revoked above).
    let _ = super::concurrency::revoke_lease_by_token_hash(&state.db, req.tenant_id, &hash).await;

    state
        .metrics
        .auth_logout_total
        .with_label_values(&["success", "ok"])
        .inc();

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}
