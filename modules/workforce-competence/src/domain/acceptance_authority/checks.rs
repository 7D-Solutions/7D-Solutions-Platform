//! Authorization queries for acceptance authority.

use sqlx::PgPool;

use crate::domain::guards::guard_non_empty;
use crate::domain::service::ServiceError;

use super::{AcceptanceAuthorityQuery, AcceptanceAuthorityResult, AuthLookupRow, ExistingRow};

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
    .bind(&query.tenant_id)
    .bind(query.operator_id)
    .bind(&query.capability_scope)
    .bind(query.at_time)
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        return Ok(AcceptanceAuthorityResult {
            allowed: true,
            operator_id: query.operator_id,
            capability_scope: query.capability_scope.clone(),
            at_time: query.at_time,
            authority_id: Some(r.id),
            effective_until: r.effective_until,
            denial_reason: None,
        });
    }

    let denial_reason = denial_reason(pool, query).await?;
    Ok(AcceptanceAuthorityResult {
        allowed: false,
        operator_id: query.operator_id,
        capability_scope: query.capability_scope.clone(),
        at_time: query.at_time,
        authority_id: None,
        effective_until: None,
        denial_reason: Some(denial_reason),
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
    .bind(&q.tenant_id)
    .bind(q.operator_id)
    .bind(&q.capability_scope)
    .fetch_optional(pool)
    .await?;

    Ok(match existing {
        None => "no_authority_found".into(),
        Some(r) if r.is_revoked => "authority_revoked".into(),
        Some(r) if r.effective_from > q.at_time => "not_yet_effective".into(),
        Some(r) if r.effective_until.is_some_and(|u| u <= q.at_time) => "authority_expired".into(),
        _ => "no_authority_found".into(),
    })
}
