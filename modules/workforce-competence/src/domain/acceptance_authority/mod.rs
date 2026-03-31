//! Acceptance authority register.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Invariants:
//! - All mutations are tenant-scoped
//! - Grants are time-bounded, revocable, auditable
//! - Idempotency key prevents double-processing on retry

pub mod checks;
pub mod grants;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use checks::check_acceptance_authority;
pub use grants::{grant_acceptance_authority, revoke_acceptance_authority};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// -- Internal row types (shared across submodules) ---------------------------

#[derive(sqlx::FromRow)]
pub(super) struct IdempotencyRecord {
    pub(super) response_body: String,
    pub(super) request_hash: String,
}

#[derive(sqlx::FromRow)]
pub(super) struct AuthLookupRow {
    pub(super) id: Uuid,
    pub(super) effective_until: Option<chrono::DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
pub(super) struct ExistingRow {
    #[allow(dead_code)]
    pub(super) id: Uuid,
    pub(super) is_revoked: bool,
    pub(super) effective_until: Option<chrono::DateTime<Utc>>,
    pub(super) effective_from: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
pub(super) struct FullRow {
    pub(super) id: Uuid,
    pub(super) tenant_id: String,
    pub(super) operator_id: Uuid,
    pub(super) capability_scope: String,
    pub(super) constraints: Option<serde_json::Value>,
    pub(super) effective_from: chrono::DateTime<Utc>,
    pub(super) effective_until: Option<chrono::DateTime<Utc>>,
    pub(super) granted_by: Option<String>,
    pub(super) is_revoked: bool,
    pub(super) revoked_at: Option<chrono::DateTime<Utc>>,
    pub(super) revocation_reason: Option<String>,
    pub(super) created_at: chrono::DateTime<Utc>,
    pub(super) updated_at: chrono::DateTime<Utc>,
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
