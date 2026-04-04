//! Delegation rules — persisted, audited actor delegation.
//!
//! Invariants:
//! - Only one active delegation per (delegator, delegatee, definition, entity_type) per tenant.
//! - Delegation is time-bounded: valid_from/valid_until.
//! - Revocation is audited: revoked_at, revoked_by, revoke_reason.
//! - Resolution: given an actor + context, return the effective actor (self or delegatee).
//! - Guard→Mutation→Outbox for create and revoke.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use super::delegation_repo::DelegationRepo;

// ── Domain model ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DelegationRule {
    pub id: Uuid,
    pub tenant_id: String,
    pub delegator_id: Uuid,
    pub delegatee_id: Uuid,
    pub definition_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub reason: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<Uuid>,
    pub revoke_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateDelegationRequest {
    pub tenant_id: String,
    pub delegator_id: Uuid,
    pub delegatee_id: Uuid,
    pub definition_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub reason: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeDelegationRequest {
    pub tenant_id: String,
    pub revoked_by: Uuid,
    pub revoke_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveDelegationQuery {
    pub tenant_id: String,
    pub actor_id: Uuid,
    pub definition_id: Option<Uuid>,
    pub entity_type: Option<String>,
}

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum DelegationError {
    #[error("Delegation not found")]
    NotFound,

    #[error("Delegation already revoked")]
    AlreadyRevoked,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Duplicate active delegation")]
    Duplicate,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
