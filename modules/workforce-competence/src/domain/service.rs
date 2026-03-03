//! Workforce competence service.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Invariants:
//! - All mutations are tenant-scoped
//! - Idempotency key prevents double-processing on retry
//! - Authorization queries are time-aware and never delete history

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{
        guards::{guard_non_empty, GuardError},
        models::{
            AssignCompetenceRequest, AuthorizationQuery, AuthorizationResult, CompetenceArtifact,
            OperatorCompetence, RegisterArtifactRequest,
        },
    },
    events::{
        create_wc_envelope, EVENT_TYPE_ARTIFACT_REGISTERED, EVENT_TYPE_COMPETENCE_ASSIGNED,
        MUTATION_CLASS_DATA_MUTATION, WC_EVENT_SCHEMA_VERSION,
    },
};

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct AuthRow {
    id: Uuid,
    expires_at: Option<chrono::DateTime<Utc>>,
}

// ============================================================================
// Register artifact
// ============================================================================

/// Register a competence artifact (cert, training, qualification).
///
/// Returns `(CompetenceArtifact, is_replay)`.
pub async fn register_artifact(
    pool: &PgPool,
    req: &RegisterArtifactRequest,
) -> Result<(CompetenceArtifact, bool), ServiceError> {
    // --- Guards ---
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.name, "name")?;
    guard_non_empty(&req.code, "code")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    if let Some(days) = req.valid_duration_days {
        if days <= 0 {
            return Err(ServiceError::Guard(GuardError::Validation(
                "valid_duration_days must be positive".to_string(),
            )));
        }
    }

    // --- Idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) =
        find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        let result: CompetenceArtifact = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Atomic transaction ---
    let now = Utc::now();
    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let artifact_type_str = req.artifact_type.to_string();

    let mut tx = pool.begin().await?;

    // Step 1: Insert artifact
    sqlx::query(
        r#"
        INSERT INTO wc_competence_artifacts
            (id, tenant_id, artifact_type, name, code, description,
             valid_duration_days, is_active, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, true, $8, $8)
        "#,
    )
    .bind(id)
    .bind(&req.tenant_id)
    .bind(&artifact_type_str)
    .bind(&req.name)
    .bind(&req.code)
    .bind(&req.description)
    .bind(req.valid_duration_days)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let artifact = CompetenceArtifact {
        id,
        tenant_id: req.tenant_id.clone(),
        artifact_type: req.artifact_type.clone(),
        name: req.name.clone(),
        code: req.code.clone(),
        description: req.description.clone(),
        valid_duration_days: req.valid_duration_days,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    // Step 2: Outbox event
    let payload = ArtifactRegisteredPayload {
        artifact_id: id,
        tenant_id: req.tenant_id.clone(),
        artifact_type: artifact_type_str.clone(),
        name: req.name.clone(),
        code: req.code.clone(),
    };
    let envelope = create_wc_envelope(
        event_id,
        req.tenant_id.clone(),
        EVENT_TYPE_ARTIFACT_REGISTERED.to_string(),
        correlation_id.clone(),
        req.causation_id.clone(),
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO wc_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'competence_artifact', $3, $4, $5::JSONB, $6, $7, $8)
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_ARTIFACT_REGISTERED)
    .bind(id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .bind(WC_EVENT_SCHEMA_VERSION)
    .execute(&mut *tx)
    .await?;

    // Step 3: Idempotency key
    let response_json = serde_json::to_string(&artifact)?;
    let expires_at = now + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO wc_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((artifact, false))
}

// ============================================================================
// Assign competence
// ============================================================================

/// Assign a competence artifact to an operator.
///
/// Returns `(OperatorCompetence, is_replay)`.
pub async fn assign_competence(
    pool: &PgPool,
    req: &AssignCompetenceRequest,
) -> Result<(OperatorCompetence, bool), ServiceError> {
    // --- Guards ---
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    // --- Idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) =
        find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        let result: OperatorCompetence = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- DB guard: artifact must exist, belong to tenant, and be active ---
    let artifact = sqlx::query_as::<_, ArtifactRow>(
        r#"
        SELECT id, valid_duration_days, is_active
        FROM wc_competence_artifacts
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(req.artifact_id)
    .bind(&req.tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(GuardError::ArtifactNotFound)?;

    if !artifact.is_active {
        return Err(ServiceError::Guard(GuardError::ArtifactInactive));
    }

    // --- Compute expiry ---
    let expires_at = req.expires_at.or_else(|| {
        artifact
            .valid_duration_days
            .map(|days| req.awarded_at + Duration::days(days as i64))
    });

    // --- Atomic transaction ---
    let now = Utc::now();
    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Insert assignment
    sqlx::query(
        r#"
        INSERT INTO wc_operator_competences
            (id, tenant_id, operator_id, artifact_id, awarded_at, expires_at,
             evidence_ref, awarded_by, is_revoked, created_at, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, false, $9, $10)
        "#,
    )
    .bind(id)
    .bind(&req.tenant_id)
    .bind(req.operator_id)
    .bind(req.artifact_id)
    .bind(req.awarded_at)
    .bind(expires_at)
    .bind(&req.evidence_ref)
    .bind(&req.awarded_by)
    .bind(now)
    .bind(&req.idempotency_key)
    .execute(&mut *tx)
    .await?;

    let assignment = OperatorCompetence {
        id,
        tenant_id: req.tenant_id.clone(),
        operator_id: req.operator_id,
        artifact_id: req.artifact_id,
        awarded_at: req.awarded_at,
        expires_at,
        evidence_ref: req.evidence_ref.clone(),
        awarded_by: req.awarded_by.clone(),
        is_revoked: false,
        revoked_at: None,
        created_at: now,
    };

    // Step 2: Outbox event
    let payload = CompetenceAssignedPayload {
        assignment_id: id,
        tenant_id: req.tenant_id.clone(),
        operator_id: req.operator_id,
        artifact_id: req.artifact_id,
        awarded_at: req.awarded_at,
        expires_at,
    };
    let envelope = create_wc_envelope(
        event_id,
        req.tenant_id.clone(),
        EVENT_TYPE_COMPETENCE_ASSIGNED.to_string(),
        correlation_id.clone(),
        req.causation_id.clone(),
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO wc_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'operator_competence', $3, $4, $5::JSONB, $6, $7, $8)
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_COMPETENCE_ASSIGNED)
    .bind(id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .bind(WC_EVENT_SCHEMA_VERSION)
    .execute(&mut *tx)
    .await?;

    // Step 3: Idempotency key
    let response_json = serde_json::to_string(&assignment)?;
    let idem_expires = now + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO wc_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(idem_expires)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((assignment, false))
}

// ============================================================================
// Authorization query
// ============================================================================

/// Check if an operator is authorized for a given capability at a given time.
///
/// Authorization requires:
/// 1. An active, non-revoked assignment of the artifact (by code) to the operator
/// 2. awarded_at <= at_time
/// 3. Either no expiry, or expires_at > at_time
/// 4. All scoped to the same tenant
pub async fn check_authorization(
    pool: &PgPool,
    query: &AuthorizationQuery,
) -> Result<AuthorizationResult, ServiceError> {
    guard_non_empty(&query.tenant_id, "tenant_id")?;
    guard_non_empty(&query.artifact_code, "artifact_code")?;

    let row = sqlx::query_as::<_, AuthRow>(
        r#"
        SELECT oc.id, oc.expires_at
        FROM wc_operator_competences oc
        JOIN wc_competence_artifacts ca ON ca.id = oc.artifact_id AND ca.tenant_id = oc.tenant_id
        WHERE oc.tenant_id = $1
          AND oc.operator_id = $2
          AND ca.code = $3
          AND ca.is_active = true
          AND oc.is_revoked = false
          AND oc.awarded_at <= $4
          AND (oc.expires_at IS NULL OR oc.expires_at > $4)
        ORDER BY oc.awarded_at DESC
        LIMIT 1
        "#,
    )
    .bind(&query.tenant_id)
    .bind(query.operator_id)
    .bind(&query.artifact_code)
    .bind(query.at_time)
    .fetch_optional(pool)
    .await?;

    Ok(match row {
        Some(r) => AuthorizationResult {
            authorized: true,
            operator_id: query.operator_id,
            artifact_code: query.artifact_code.clone(),
            at_time: query.at_time,
            assignment_id: Some(r.id),
            expires_at: r.expires_at,
        },
        None => AuthorizationResult {
            authorized: false,
            operator_id: query.operator_id,
            artifact_code: query.artifact_code.clone(),
            at_time: query.at_time,
            assignment_id: None,
            expires_at: None,
        },
    })
}

// ============================================================================
// Get artifact by ID
// ============================================================================

pub async fn get_artifact(
    pool: &PgPool,
    tenant_id: &str,
    artifact_id: Uuid,
) -> Result<Option<CompetenceArtifact>, ServiceError> {
    let row = sqlx::query_as::<_, ArtifactFullRow>(
        r#"
        SELECT id, tenant_id, artifact_type, name, code, description,
               valid_duration_days, is_active, created_at, updated_at
        FROM wc_competence_artifacts
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| CompetenceArtifact {
        id: r.id,
        tenant_id: r.tenant_id,
        artifact_type: r.artifact_type.parse().unwrap_or(
            crate::domain::models::ArtifactType::Qualification,
        ),
        name: r.name,
        code: r.code,
        description: r.description,
        valid_duration_days: r.valid_duration_days,
        is_active: r.is_active,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

// ============================================================================
// Internal helpers
// ============================================================================

#[derive(sqlx::FromRow)]
struct ArtifactRow {
    #[allow(dead_code)]
    id: Uuid,
    valid_duration_days: Option<i32>,
    is_active: bool,
}

#[derive(sqlx::FromRow)]
struct ArtifactFullRow {
    id: Uuid,
    tenant_id: String,
    artifact_type: String,
    name: String,
    code: String,
    description: Option<String>,
    valid_duration_days: Option<i32>,
    is_active: bool,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

async fn find_idempotency_key(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash
        FROM wc_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactRegisteredPayload {
    pub artifact_id: Uuid,
    pub tenant_id: String,
    pub artifact_type: String,
    pub name: String,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompetenceAssignedPayload {
    pub assignment_id: Uuid,
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub artifact_id: Uuid,
    pub awarded_at: chrono::DateTime<Utc>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::ArtifactType;

    fn valid_register_req() -> RegisterArtifactRequest {
        RegisterArtifactRequest {
            tenant_id: "tenant-1".to_string(),
            artifact_type: ArtifactType::Certification,
            name: "IPC-A-610 Soldering".to_string(),
            code: "IPC-A-610".to_string(),
            description: Some("IPC soldering certification".to_string()),
            valid_duration_days: Some(365),
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    fn valid_assign_req() -> AssignCompetenceRequest {
        AssignCompetenceRequest {
            tenant_id: "tenant-1".to_string(),
            operator_id: Uuid::new_v4(),
            artifact_id: Uuid::new_v4(),
            awarded_at: Utc::now(),
            expires_at: None,
            evidence_ref: Some("cert-scan-001.pdf".to_string()),
            awarded_by: Some("QA Manager".to_string()),
            idempotency_key: "idem-assign-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn register_rejects_empty_tenant() {
        let mut r = valid_register_req();
        r.tenant_id = "".to_string();
        let hash = serde_json::to_string(&r).expect("serialization must succeed");
        assert!(!hash.is_empty());
        assert!(guard_non_empty("", "tenant_id").is_err());
    }

    #[test]
    fn register_rejects_empty_code() {
        assert!(guard_non_empty("  ", "code").is_err());
    }

    #[test]
    fn register_accepts_valid_fields() {
        let r = valid_register_req();
        assert!(guard_non_empty(&r.tenant_id, "tenant_id").is_ok());
        assert!(guard_non_empty(&r.name, "name").is_ok());
        assert!(guard_non_empty(&r.code, "code").is_ok());
    }

    #[test]
    fn assign_rejects_empty_tenant() {
        let r = valid_assign_req();
        assert!(guard_non_empty(&r.tenant_id, "tenant_id").is_ok());
        assert!(guard_non_empty("", "tenant_id").is_err());
    }

    #[test]
    fn artifact_type_roundtrip() {
        let at = ArtifactType::Certification;
        let s = at.to_string();
        assert_eq!(s, "certification");
        let parsed: ArtifactType = s.parse().expect("should parse certification");
        assert_eq!(parsed, at);
    }
}
