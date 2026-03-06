use crate::{
    clients::tenant_registry::{TenantGate, TenantRegistryClient},
    events::publisher::EventPublisher,
    metrics::Metrics,
    middleware::tracing::get_trace_id_from_extensions,
    rate_limit::KeyedLimiters,
};
use event_bus::EventEnvelope;
use axum::{
    extract::{Path, State},
    http::{Extensions, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

use super::{
    concurrency::HashConcurrencyLimiter,
    jwt::{self, JwtKeys},
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
    pub forgot_per_min_per_email: u32,
    pub forgot_per_min_per_ip: u32,
    pub reset_per_min_per_ip: u32,

    // Password reset TTL
    pub password_reset_ttl_minutes: i64,

    // DB-backed concurrent seat limit per tenant (static fallback when registry unavailable)
    pub max_concurrent_sessions: i64,

    // Entitlement client — fetches concurrent_user_limit from tenant-registry with TTL cache.
    // None means use max_concurrent_sessions unconditionally.
    pub tenant_registry: Option<TenantRegistryClient>,
}

pub(super) type ApiErr = (StatusCode, HeaderMap, String);

pub(super) fn err(code: StatusCode, msg: impl Into<String>) -> ApiErr {
    (code, HeaderMap::new(), msg.into())
}

pub(super) fn err_retry_after(
    code: StatusCode,
    wait: std::time::Duration,
    msg: impl Into<String>,
) -> ApiErr {
    let mut headers = HeaderMap::new();
    let secs = wait.as_secs().max(1).to_string();
    headers.insert(
        "Retry-After",
        HeaderValue::from_str(&secs).unwrap_or(HeaderValue::from_static("1")),
    );
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

#[derive(Debug, Deserialize)]
pub struct AccessReviewReq {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub review_id: Uuid,
    pub reviewed_by: Uuid,
    pub decision: String,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
    pub causation_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SodPolicyUpsertReq {
    pub tenant_id: Uuid,
    pub action_key: String,
    pub primary_role_id: Uuid,
    pub conflicting_role_id: Uuid,
    pub allow_override: bool,
    pub override_requires_approval: bool,
    pub actor_user_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
    pub causation_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SodEvaluateReq {
    pub tenant_id: Uuid,
    pub action_key: String,
    pub actor_user_id: Uuid,
    pub subject_user_id: Option<Uuid>,
    pub override_granted_by: Option<Uuid>,
    pub override_ticket: Option<String>,
    pub idempotency_key: Option<String>,
    pub causation_id: Option<Uuid>,
}

pub async fn record_access_review(
    State(state): State<Arc<AuthState>>,
    extensions: Extensions,
    Json(req): Json<AccessReviewReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);
    let decision = req.decision.trim();
    if decision.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "decision is required"));
    }

    let idempotency_key = req
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "access-review:{}:{}:{}",
                req.tenant_id, req.user_id, req.review_id
            )
        });

    let ctx = crate::db::user_lifecycle_audit::LifecycleAuditContext {
        producer: state.producer.clone(),
        trace_id,
        causation_id: req.causation_id,
        idempotency_key,
    };

    crate::db::user_lifecycle_audit::record_access_review_decision(
        &state.db,
        req.tenant_id,
        req.user_id,
        req.reviewed_by,
        decision,
        req.review_id,
        req.notes.as_deref(),
        &ctx,
    )
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("audit write: {e}"),
        )
    })?;

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}

pub async fn upsert_sod_policy(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(req): Json<SodPolicyUpsertReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);
    if req.action_key.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "action_key is required"));
    }

    let idempotency_key = req
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-idempotency-key")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| {
            format!(
                "sod-policy:{}:{}:{}:{}",
                req.tenant_id, req.action_key, req.primary_role_id, req.conflicting_role_id
            )
        });

    let result = crate::db::sod::upsert_policy(
        &state.db,
        crate::db::sod::SodPolicyUpsert {
            tenant_id: req.tenant_id,
            action_key: req.action_key,
            primary_role_id: req.primary_role_id,
            conflicting_role_id: req.conflicting_role_id,
            allow_override: req.allow_override,
            override_requires_approval: req.override_requires_approval,
            actor_user_id: req.actor_user_id,
            idempotency_key,
            trace_id,
            causation_id: req.causation_id,
            producer: state.producer.clone(),
        },
    )
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("sod policy write: {e}"),
        )
    })?;

    Ok((StatusCode::OK, Json(result)))
}

pub async fn evaluate_sod(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(req): Json<SodEvaluateReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);
    if req.action_key.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "action_key is required"));
    }

    let idempotency_key = req
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-idempotency-key")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| {
            format!(
                "sod-eval:{}:{}:{}:{}",
                req.tenant_id,
                req.action_key,
                req.actor_user_id,
                req.subject_user_id
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| "none".to_string())
            )
        });

    let result = crate::db::sod::evaluate_decision(
        &state.db,
        crate::db::sod::SodDecisionRequest {
            tenant_id: req.tenant_id,
            action_key: req.action_key,
            actor_user_id: req.actor_user_id,
            subject_user_id: req.subject_user_id,
            override_granted_by: req.override_granted_by,
            override_ticket: req.override_ticket,
            idempotency_key,
            trace_id,
            causation_id: req.causation_id,
            producer: state.producer.clone(),
        },
    )
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("sod evaluate: {e}"),
        )
    })?;

    Ok((StatusCode::OK, Json(result)))
}

pub async fn list_sod_policies(
    State(state): State<Arc<AuthState>>,
    Path((tenant_id, action_key)): Path<(Uuid, String)>,
) -> Result<impl IntoResponse, ApiErr> {
    let policies = crate::db::sod::list_policies(&state.db, tenant_id, &action_key)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("sod list: {e}")))?;
    Ok((StatusCode::OK, Json(policies)))
}

pub async fn delete_sod_policy(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    extensions: Extensions,
    Path((tenant_id, rule_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let idempotency_key = headers
        .get("x-idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("sod-delete:{tenant_id}:{rule_id}"));

    let result = crate::db::sod::delete_policy(
        &state.db,
        crate::db::sod::SodPolicyDeleteRequest {
            tenant_id,
            policy_id: rule_id,
            actor_user_id: None,
            idempotency_key,
            trace_id,
            causation_id: None,
            producer: state.producer.clone(),
        },
    )
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("sod delete: {e}"),
        )
    })?;

    if result.idempotent_replay || result.deleted {
        Ok((StatusCode::OK, Json(serde_json::json!({"ok": true, "deleted": result.deleted, "idempotent_replay": result.idempotent_replay}))))
    } else {
        Err(err(StatusCode::NOT_FOUND, "policy not found"))
    }
}

pub async fn get_user_lifecycle_timeline(
    State(state): State<Arc<AuthState>>,
    Path((tenant_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiErr> {
    let timeline = crate::db::user_lifecycle_audit::list_user_lifecycle_timeline(
        &state.db, tenant_id, user_id,
    )
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("timeline read: {e}"),
        )
    })?;

    Ok((StatusCode::OK, Json(timeline)))
}

pub async fn register(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(req): Json<RegisterReq>,
) -> Result<impl IntoResponse, ApiErr> {
    let trace_id = get_trace_id_from_extensions(&extensions);

    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        state
            .metrics
            .auth_register_total
            .with_label_values(&["failure", "invalid_email"])
            .inc();
        return Err(err(StatusCode::BAD_REQUEST, "invalid email"));
    }

    // Password policy validation FIRST (before rate limit, before hash)
    let rules = PasswordRules::default();
    if let Err(e) = validate_password(&rules, &req.password) {
        state
            .metrics
            .auth_register_total
            .with_label_values(&["failure", "weak_password"])
            .inc();
        return Err(err(StatusCode::BAD_REQUEST, e.to_string()));
    }

    // keyed limit BEFORE hashing
    if let Err(wait) = state.keyed_limits.check_register_email(
        &req.tenant_id.to_string(),
        &email,
        state.register_per_min_per_email,
    ) {
        state
            .metrics
            .auth_rate_limited_total
            .with_label_values(&["email"])
            .inc();
        state
            .metrics
            .auth_register_total
            .with_label_values(&["failure", "rate_limited"])
            .inc();
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
            state
                .metrics
                .auth_register_total
                .with_label_values(&["failure", "hash_busy"])
                .inc();
            tracing::warn!(tenant_id=%req.tenant_id, trace_id=%trace_id, "auth.hash_busy");
            return Err(err(StatusCode::SERVICE_UNAVAILABLE, "auth busy"));
        }
    };

    let hash =
        hash_password(&state.pwd, &req.password).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

    let idempotency_key = headers
        .get("x-idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("register:{}:{}", req.tenant_id, req.user_id));

    let mut tx = state.db.begin().await.map_err(|e| {
        state
            .metrics
            .auth_register_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

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
    .execute(&mut *tx)
    .await;

    match res {
        Ok(_) => {
            #[derive(Serialize)]
            struct Data {
                user_id: String,
                email: String,
            }

            let payload = serde_json::to_value(Data {
                user_id: req.user_id.to_string(),
                email: email.clone(),
            })
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("serialize: {e}")))?;

            let ctx = crate::db::user_lifecycle_audit::LifecycleAuditContext {
                producer: state.producer.clone(),
                trace_id: trace_id.clone(),
                causation_id: None,
                idempotency_key: idempotency_key.clone(),
            };

            crate::db::user_lifecycle_audit::append_lifecycle_event_tx(
                &mut tx,
                req.tenant_id,
                req.user_id,
                crate::db::user_lifecycle_audit::LifecycleEventType::UserCreated,
                None,
                None,
                None,
                None,
                payload,
                &ctx,
            )
            .await
            .map_err(|e| {
                state
                    .metrics
                    .auth_register_total
                    .with_label_values(&["failure", "db_error"])
                    .inc();
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("audit write: {e}"),
                )
            })?;

            tx.commit().await.map_err(|e| {
                state
                    .metrics
                    .auth_register_total
                    .with_label_values(&["failure", "db_error"])
                    .inc();
                err(StatusCode::INTERNAL_SERVER_ERROR, format!("tx commit: {e}"))
            })?;

            state
                .metrics
                .auth_register_total
                .with_label_values(&["success", "ok"])
                .inc();

            let env = EventEnvelope::new(
                req.tenant_id.to_string(),
                state.producer.clone(),
                "auth.user.registered".to_string(),
                Data {
                    user_id: req.user_id.to_string(),
                    email: email.clone(),
                },
            )
            .with_schema_version("auth.user.registered/v1".to_string())
            .with_trace_id(Some(trace_id))
            .with_mutation_class(Some("user-data".to_string()));

            if state
                .events
                .publish(
                    "auth.events.user.registered",
                    "auth.user.registered.v1.json",
                    &env,
                )
                .await
                .is_err()
            {
                state
                    .metrics
                    .auth_nats_publish_fail_total
                    .with_label_values(&["auth.user.registered"])
                    .inc();
            }

            Ok((StatusCode::OK, Json(OkResponse { ok: true })))
        }
        Err(e) => {
            let _ = tx.rollback().await;
            state
                .metrics
                .auth_register_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            if let Some(db_err) = e.as_database_error() {
                if db_err.code().as_deref() == Some("23505") {
                    return Err(err(StatusCode::CONFLICT, "credential already exists"));
                }
            }
            Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("db error: {e}"),
            ))
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
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "invalid_email"])
            .inc();
        return Err(err(StatusCode::BAD_REQUEST, "invalid email"));
    }

    // keyed limit BEFORE hashing
    if let Err(wait) = state.keyed_limits.check_login_email(
        &req.tenant_id.to_string(),
        &email,
        state.login_per_min_per_email,
    ) {
        state
            .metrics
            .auth_rate_limited_total
            .with_label_values(&["email"])
            .inc();
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "rate_limited"])
            .inc();
        return Err(err_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            wait,
            "rate limited",
        ));
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
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "not_found"])
                .inc();
            return Err(err(StatusCode::UNAUTHORIZED, "invalid credentials"));
        }
    };

    let user_id: Uuid = row.get("user_id");
    let password_hash: String = row.get("password_hash");
    let is_active: bool = row.get("is_active");
    let lock_until: Option<chrono::DateTime<Utc>> = row.get("lock_until");

    if !is_active {
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "inactive"])
            .inc();
        return Err(err(StatusCode::FORBIDDEN, "account inactive"));
    }

    if let Some(lu) = lock_until {
        if lu > Utc::now() {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "locked"])
                .inc();
            return Err(err(StatusCode::LOCKED, "account temporarily locked"));
        }
    }

    // Acquire semaphore permit for password verification
    let _permit = match state.hash_limiter.acquire().await {
        Ok(p) => p,
        Err(_) => {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "hash_busy"])
                .inc();
            tracing::warn!(tenant_id=%req.tenant_id, email=%email, trace_id=%trace_id, "auth.hash_busy");
            return Err(err(StatusCode::SERVICE_UNAVAILABLE, "auth busy"));
        }
    };

    let t = Metrics::timer();
    let ok = verify_password(&state.pwd, &req.password, &password_hash).map_err(|e| {
        state
            .metrics
            .auth_password_verify_duration_seconds
            .with_label_values(&["error"])
            .observe(t.elapsed().as_secs_f64());
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "verify_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    if !ok {
        state
            .metrics
            .auth_password_verify_duration_seconds
            .with_label_values(&["fail"])
            .observe(t.elapsed().as_secs_f64());
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "invalid_password"])
            .inc();

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

    state
        .metrics
        .auth_password_verify_duration_seconds
        .with_label_values(&["ok"])
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

    // Resolve RBAC roles and effective permissions for token embedding
    let roles = crate::db::rbac::list_roles_for_user(&state.db, req.tenant_id, user_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_login_total
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
                .auth_login_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rbac perms: {e}"),
            )
        })?;

    let role_snapshot_id = jwt::compute_role_snapshot_id(&roles);

    let refresh_raw = generate_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_raw);
    let expires_at = Utc::now() + chrono::Duration::days(state.refresh_ttl_days);

    // Atomic seat-limit enforcement: advisory lock + count + insert in one transaction.
    let mut tx = state.db.begin().await.map_err(|e| {
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?;

    super::concurrency::acquire_tenant_xact_lock(&mut tx, req.tenant_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("advisory lock: {e}"),
            )
        })?;

    // Tenant lifecycle gate: deny login for suspended/canceled tenants; deny new login for past_due.
    if let Some(client) = &state.tenant_registry {
        match client.get_tenant_gate(req.tenant_id, &state.metrics).await {
            Ok(TenantGate::Allow) => {}
            Ok(TenantGate::DenyNewLogin { status }) => {
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
                    "auth.tenant_status_deny_new_login"
                );
                let _ = tx.rollback().await;
                return Err(err(StatusCode::FORBIDDEN, "tenant account past due"));
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
                    "auth.tenant_status_denied"
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
                    "auth.tenant_status_unavailable_deny"
                );
                let _ = tx.rollback().await;
                return Err(err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "tenant status service unavailable",
                ));
            }
        }
    }

    let active = super::concurrency::count_active_leases_in_tx(&mut tx, req.tenant_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("seat count: {e}"),
            )
        })?;

    // Resolve the per-tenant concurrent session limit from the entitlement client
    // (with TTL cache).  Fall back to the static config limit when the client is
    // not configured.  Fail-closed: if the client cannot determine the limit and
    // has no usable cached value, deny the login.
    let seat_limit = match &state.tenant_registry {
        Some(client) => {
            match client
                .get_concurrent_user_limit(req.tenant_id, &state.metrics)
                .await
            {
                Ok(limit) => limit,
                Err(_) => {
                    state
                        .metrics
                        .auth_login_total
                        .with_label_values(&["failure", "entitlement_unavailable"])
                        .inc();
                    tracing::warn!(
                        tenant_id = %req.tenant_id,
                        user_id = %user_id,
                        trace_id = %trace_id,
                        "auth.entitlement_unavailable_deny"
                    );
                    let _ = tx.rollback().await;
                    return Err(err(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "entitlement service unavailable",
                    ));
                }
            }
        }
        None => state.max_concurrent_sessions,
    };

    if active >= seat_limit {
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "seat_limit"])
            .inc();
        tracing::warn!(
            tenant_id = %req.tenant_id,
            user_id = %user_id,
            active_seats = active,
            limit = seat_limit,
            trace_id = %trace_id,
            "auth.seat_limit_exceeded"
        );
        let _ = tx.rollback().await;
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "concurrent session limit reached",
        ));
    }

    let new_token_id: Uuid = sqlx::query(
        r#"
        INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(req.tenant_id)
    .bind(user_id)
    .bind(&refresh_hash)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
    })?
    .get("id");

    super::concurrency::create_lease_in_tx(&mut tx, req.tenant_id, user_id, new_token_id)
        .await
        .map_err(|e| {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "db_error"])
                .inc();
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create lease: {e}"),
            )
        })?;

    let access = state
        .jwt
        .sign_access_token_enriched(
            req.tenant_id,
            user_id,
            roles,
            perms,
            jwt::actor_type::USER,
            state.access_ttl_minutes,
            Some(new_token_id),
            Some(role_snapshot_id),
        )
        .map_err(|e| {
            state
                .metrics
                .auth_login_total
                .with_label_values(&["failure", "token_sign_error"])
                .inc();
            err(StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    tx.commit().await.map_err(|e| {
        state
            .metrics
            .auth_login_total
            .with_label_values(&["failure", "db_error"])
            .inc();
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("tx commit: {e}"))
    })?;

    state
        .metrics
        .auth_login_total
        .with_label_values(&["success", "ok"])
        .inc();

    #[derive(Serialize)]
    struct Data {
        user_id: String,
    }
    let env = EventEnvelope::new(
        req.tenant_id.to_string(),
        state.producer.clone(),
        "auth.user.logged_in".to_string(),
        Data {
            user_id: user_id.to_string(),
        },
    )
    .with_schema_version("auth.user.logged_in/v1".to_string())
    .with_trace_id(Some(trace_id))
    .with_actor(user_id, "User".to_string())
    .with_mutation_class(Some("user-data".to_string()));

    if state
        .events
        .publish(
            "auth.events.user.logged_in",
            "auth.user.logged_in.v1.json",
            &env,
        )
        .await
        .is_err()
    {
        state
            .metrics
            .auth_nats_publish_fail_total
            .with_label_values(&["auth.user.logged_in"])
            .inc();
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
