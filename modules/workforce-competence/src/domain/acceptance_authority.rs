//! Acceptance authority register.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Invariants:
//! - All mutations are tenant-scoped
//! - Grants are time-bounded, revocable, auditable
//! - Idempotency key prevents double-processing on retry

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::guards::{guard_non_empty, GuardError};
use super::service::ServiceError;
use crate::events::{
    create_wc_envelope, EVENT_TYPE_AUTHORITY_GRANTED, EVENT_TYPE_AUTHORITY_REVOKED,
    MUTATION_CLASS_DATA_MUTATION, WC_EVENT_SCHEMA_VERSION,
};

// -- Models ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceAuthority {
    pub id: Uuid,
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub constraints: Option<serde_json::Value>,
    pub effective_from: chrono::DateTime<Utc>,
    pub effective_until: Option<chrono::DateTime<Utc>>,
    pub granted_by: Option<String>,
    pub is_revoked: bool,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
    pub revocation_reason: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GrantAuthorityRequest {
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub constraints: Option<serde_json::Value>,
    pub effective_from: chrono::DateTime<Utc>,
    pub effective_until: Option<chrono::DateTime<Utc>>,
    pub granted_by: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevokeAuthorityRequest {
    pub tenant_id: String,
    pub authority_id: Uuid,
    pub revocation_reason: String,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AcceptanceAuthorityQuery {
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub at_time: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AcceptanceAuthorityResult {
    pub allowed: bool,
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub at_time: chrono::DateTime<Utc>,
    pub authority_id: Option<Uuid>,
    pub effective_until: Option<chrono::DateTime<Utc>>,
    pub denial_reason: Option<String>,
}

// -- Internal row types ------------------------------------------------------

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct AuthLookupRow {
    id: Uuid,
    effective_until: Option<chrono::DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct ExistingRow {
    #[allow(dead_code)]
    id: Uuid,
    is_revoked: bool,
    effective_until: Option<chrono::DateTime<Utc>>,
    effective_from: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct FullRow {
    id: Uuid,
    tenant_id: String,
    operator_id: Uuid,
    capability_scope: String,
    constraints: Option<serde_json::Value>,
    effective_from: chrono::DateTime<Utc>,
    effective_until: Option<chrono::DateTime<Utc>>,
    granted_by: Option<String>,
    is_revoked: bool,
    revoked_at: Option<chrono::DateTime<Utc>>,
    revocation_reason: Option<String>,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

impl From<FullRow> for AcceptanceAuthority {
    fn from(r: FullRow) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id,
            operator_id: r.operator_id,
            capability_scope: r.capability_scope,
            constraints: r.constraints,
            effective_from: r.effective_from,
            effective_until: r.effective_until,
            granted_by: r.granted_by,
            is_revoked: r.is_revoked,
            revoked_at: r.revoked_at,
            revocation_reason: r.revocation_reason,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

// -- Event payloads ----------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorityGrantedPayload {
    pub authority_id: Uuid,
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub effective_from: chrono::DateTime<Utc>,
    pub effective_until: Option<chrono::DateTime<Utc>>,
    pub granted_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorityRevokedPayload {
    pub authority_id: Uuid,
    pub tenant_id: String,
    pub revocation_reason: String,
    pub revoked_at: chrono::DateTime<Utc>,
}

// -- Grant -------------------------------------------------------------------

/// Grant acceptance authority. Returns `(AcceptanceAuthority, is_replay)`.
pub async fn grant_acceptance_authority(
    pool: &PgPool,
    req: &GrantAuthorityRequest,
) -> Result<(AcceptanceAuthority, bool), ServiceError> {
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.capability_scope, "capability_scope")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    if let Some(until) = req.effective_until {
        if until <= req.effective_from {
            return Err(ServiceError::Guard(GuardError::Validation(
                "effective_until must be after effective_from".to_string(),
            )));
        }
    }

    let request_hash = serde_json::to_string(req)?;
    if let Some(rec) = find_idem(pool, &req.tenant_id, &req.idempotency_key).await? {
        if rec.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        return Ok((serde_json::from_str(&rec.response_body)?, true));
    }

    let now = Utc::now();
    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let corr = req.correlation_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let constraints_json = req.constraints.as_ref().map(serde_json::to_string).transpose()?;

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO wc_acceptance_authorities
            (id, tenant_id, operator_id, capability_scope, constraints,
             effective_from, effective_until, granted_by, is_revoked,
             idempotency_key, created_at, updated_at)
         VALUES ($1,$2,$3,$4,$5::JSONB,$6,$7,$8,false,$9,$10,$10)",
    )
    .bind(id).bind(&req.tenant_id).bind(req.operator_id)
    .bind(&req.capability_scope).bind(&constraints_json)
    .bind(req.effective_from).bind(req.effective_until)
    .bind(&req.granted_by).bind(&req.idempotency_key).bind(now)
    .execute(&mut *tx).await?;

    let authority = AcceptanceAuthority {
        id, tenant_id: req.tenant_id.clone(), operator_id: req.operator_id,
        capability_scope: req.capability_scope.clone(), constraints: req.constraints.clone(),
        effective_from: req.effective_from, effective_until: req.effective_until,
        granted_by: req.granted_by.clone(), is_revoked: false,
        revoked_at: None, revocation_reason: None, created_at: now, updated_at: now,
    };

    let payload = AuthorityGrantedPayload {
        authority_id: id, tenant_id: req.tenant_id.clone(),
        operator_id: req.operator_id, capability_scope: req.capability_scope.clone(),
        effective_from: req.effective_from, effective_until: req.effective_until,
        granted_by: req.granted_by.clone(),
    };
    write_outbox_event(
        &mut tx, event_id, EVENT_TYPE_AUTHORITY_GRANTED, id,
        &req.tenant_id, &corr, &req.causation_id, payload,
    ).await?;

    write_idem_key(&mut tx, &req.tenant_id, &req.idempotency_key, &request_hash, &authority, 201, now).await?;
    tx.commit().await?;
    Ok((authority, false))
}

// -- Revoke ------------------------------------------------------------------

/// Revoke an acceptance authority. Returns `(AcceptanceAuthority, is_replay)`.
pub async fn revoke_acceptance_authority(
    pool: &PgPool,
    req: &RevokeAuthorityRequest,
) -> Result<(AcceptanceAuthority, bool), ServiceError> {
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.revocation_reason, "revocation_reason")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(rec) = find_idem(pool, &req.tenant_id, &req.idempotency_key).await? {
        if rec.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        return Ok((serde_json::from_str(&rec.response_body)?, true));
    }

    // Guard: authority must exist, belong to tenant, and not already revoked
    let existing = sqlx::query_as::<_, ExistingRow>(
        "SELECT id, is_revoked, effective_until, effective_from
         FROM wc_acceptance_authorities WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.authority_id).bind(&req.tenant_id)
    .fetch_optional(pool).await?
    .ok_or(GuardError::Validation("acceptance authority not found or wrong tenant".into()))?;

    if existing.is_revoked {
        return Err(ServiceError::Guard(GuardError::Validation(
            "acceptance authority is already revoked".into(),
        )));
    }

    let now = Utc::now();
    let event_id = Uuid::new_v4();
    let corr = req.correlation_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE wc_acceptance_authorities
         SET is_revoked = true, revoked_at = $1, revocation_reason = $2, updated_at = $1
         WHERE id = $3 AND tenant_id = $4",
    )
    .bind(now).bind(&req.revocation_reason)
    .bind(req.authority_id).bind(&req.tenant_id)
    .execute(&mut *tx).await?;

    let authority = fetch_by_id(&mut *tx, &req.tenant_id, req.authority_id).await?;

    let payload = AuthorityRevokedPayload {
        authority_id: req.authority_id, tenant_id: req.tenant_id.clone(),
        revocation_reason: req.revocation_reason.clone(), revoked_at: now,
    };
    write_outbox_event(
        &mut tx, event_id, EVENT_TYPE_AUTHORITY_REVOKED, req.authority_id,
        &req.tenant_id, &corr, &req.causation_id, payload,
    ).await?;

    write_idem_key(&mut tx, &req.tenant_id, &req.idempotency_key, &request_hash, &authority, 200, now).await?;
    tx.commit().await?;
    Ok((authority, false))
}

// -- Authorization query -----------------------------------------------------

/// Check if an operator has acceptance authority for a scope at time T.
pub async fn check_acceptance_authority(
    pool: &PgPool,
    query: &AcceptanceAuthorityQuery,
) -> Result<AcceptanceAuthorityResult, ServiceError> {
    guard_non_empty(&query.tenant_id, "tenant_id")?;
    guard_non_empty(&query.capability_scope, "capability_scope")?;

    let row = sqlx::query_as::<_, AuthLookupRow>(
        "SELECT id, effective_until FROM wc_acceptance_authorities
         WHERE tenant_id = $1 AND operator_id = $2 AND capability_scope = $3
           AND is_revoked = false AND effective_from <= $4
           AND (effective_until IS NULL OR effective_until > $4)
         ORDER BY effective_from DESC LIMIT 1",
    )
    .bind(&query.tenant_id).bind(query.operator_id)
    .bind(&query.capability_scope).bind(query.at_time)
    .fetch_optional(pool).await?;

    if let Some(r) = row {
        return Ok(AcceptanceAuthorityResult {
            allowed: true, operator_id: query.operator_id,
            capability_scope: query.capability_scope.clone(),
            at_time: query.at_time, authority_id: Some(r.id),
            effective_until: r.effective_until, denial_reason: None,
        });
    }

    let denial_reason = denial_reason(pool, query).await?;
    Ok(AcceptanceAuthorityResult {
        allowed: false, operator_id: query.operator_id,
        capability_scope: query.capability_scope.clone(),
        at_time: query.at_time, authority_id: None,
        effective_until: None, denial_reason: Some(denial_reason),
    })
}

// -- Helpers -----------------------------------------------------------------

async fn denial_reason(
    pool: &PgPool,
    q: &AcceptanceAuthorityQuery,
) -> Result<String, ServiceError> {
    let existing = sqlx::query_as::<_, ExistingRow>(
        "SELECT id, is_revoked, effective_until, effective_from
         FROM wc_acceptance_authorities
         WHERE tenant_id = $1 AND operator_id = $2 AND capability_scope = $3
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&q.tenant_id).bind(q.operator_id).bind(&q.capability_scope)
    .fetch_optional(pool).await?;

    Ok(match existing {
        None => "no_authority_found".into(),
        Some(r) if r.is_revoked => "authority_revoked".into(),
        Some(r) if r.effective_from > q.at_time => "not_yet_effective".into(),
        Some(r) if r.effective_until.is_some_and(|u| u <= q.at_time) => "authority_expired".into(),
        _ => "no_authority_found".into(),
    })
}

async fn find_idem(
    pool: &PgPool, tenant_id: &str, key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        "SELECT response_body::TEXT AS response_body, request_hash
         FROM wc_idempotency_keys WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id).bind(key).fetch_optional(pool).await
}

async fn fetch_by_id(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    tenant_id: &str, id: Uuid,
) -> Result<AcceptanceAuthority, ServiceError> {
    let row = sqlx::query_as::<_, FullRow>(
        "SELECT id, tenant_id, operator_id, capability_scope, constraints,
                effective_from, effective_until, granted_by, is_revoked,
                revoked_at, revocation_reason, created_at, updated_at
         FROM wc_acceptance_authorities WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id).bind(tenant_id).fetch_one(executor).await?;
    Ok(row.into())
}

async fn write_outbox_event<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid, event_type: &str, aggregate_id: Uuid,
    tenant_id: &str, correlation_id: &str, causation_id: &Option<String>,
    payload: T,
) -> Result<(), ServiceError> {
    let envelope = create_wc_envelope(
        event_id, tenant_id.to_string(), event_type.to_string(),
        correlation_id.to_string(), causation_id.clone(),
        MUTATION_CLASS_DATA_MUTATION.to_string(), payload,
    );
    let json = serde_json::to_string(&envelope)?;
    sqlx::query(
        "INSERT INTO wc_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
         VALUES ($1,$2,'acceptance_authority',$3,$4,$5::JSONB,$6,$7,$8)",
    )
    .bind(event_id).bind(event_type).bind(aggregate_id.to_string())
    .bind(tenant_id).bind(&json).bind(correlation_id)
    .bind(causation_id).bind(WC_EVENT_SCHEMA_VERSION)
    .execute(&mut **tx).await?;
    Ok(())
}

async fn write_idem_key<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str, key: &str, hash: &str, response: &T,
    status_code: i32, now: chrono::DateTime<Utc>,
) -> Result<(), ServiceError> {
    let json = serde_json::to_string(response)?;
    sqlx::query(
        "INSERT INTO wc_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
         VALUES ($1,$2,$3,$4::JSONB,$5,$6)",
    )
    .bind(tenant_id).bind(key).bind(hash).bind(&json)
    .bind(status_code).bind(now + Duration::days(7))
    .execute(&mut **tx).await?;
    Ok(())
}
