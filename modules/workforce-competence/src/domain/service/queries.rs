//! Read-only queries: authorization checks, artifact lookups.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::{
    guards::guard_non_empty,
    models::{AuthorizationQuery, AuthorizationResult, CompetenceArtifact},
};

use super::core::ServiceError;

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
struct AuthRow {
    id: Uuid,
    expires_at: Option<chrono::DateTime<Utc>>,
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
        artifact_type: r
            .artifact_type
            .parse()
            .unwrap_or(crate::domain::models::ArtifactType::Qualification),
        name: r.name,
        code: r.code,
        description: r.description,
        valid_duration_days: r.valid_duration_days,
        is_active: r.is_active,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}
