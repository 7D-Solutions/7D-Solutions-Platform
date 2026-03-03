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
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::{envelope, subjects};
use crate::outbox;

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

// ── Event payloads ───────────────────────────────────────────

#[derive(Debug, Serialize)]
struct DelegationCreatedPayload {
    delegation_id: Uuid,
    tenant_id: String,
    delegator_id: Uuid,
    delegatee_id: Uuid,
    definition_id: Option<Uuid>,
    entity_type: Option<String>,
    valid_from: DateTime<Utc>,
    valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct DelegationRevokedPayload {
    delegation_id: Uuid,
    tenant_id: String,
    delegator_id: Uuid,
    delegatee_id: Uuid,
    revoked_by: Uuid,
    revoke_reason: Option<String>,
}

// ── Repository ───────────────────────────────────────────────

pub struct DelegationRepo;

impl DelegationRepo {
    /// Create a delegation rule.
    /// Guard: delegator != delegatee, no active duplicate.
    /// Mutation: INSERT delegation row.
    /// Outbox: delegation.created event.
    pub async fn create(
        pool: &PgPool,
        req: &CreateDelegationRequest,
    ) -> Result<DelegationRule, DelegationError> {
        if req.delegator_id == req.delegatee_id {
            return Err(DelegationError::Validation(
                "delegator and delegatee must be different".into(),
            ));
        }

        if let (Some(from), Some(until)) = (req.valid_from, req.valid_until) {
            if until <= from {
                return Err(DelegationError::Validation(
                    "valid_until must be after valid_from".into(),
                ));
            }
        }

        let id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let valid_from = req.valid_from.unwrap_or_else(Utc::now);
        let mut tx = pool.begin().await?;

        let delegation = sqlx::query_as::<_, DelegationRule>(
            r#"
            INSERT INTO workflow_delegation_rules
                (id, tenant_id, delegator_id, delegatee_id, definition_id,
                 entity_type, reason, valid_from, valid_until)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.delegator_id)
        .bind(req.delegatee_id)
        .bind(req.definition_id)
        .bind(&req.entity_type)
        .bind(&req.reason)
        .bind(valid_from)
        .bind(req.valid_until)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err
                    .message()
                    .contains("duplicate key value violates unique constraint")
                {
                    return DelegationError::Duplicate;
                }
            }
            DelegationError::Database(e)
        })?;

        // ── Outbox: delegation.created ──
        let payload = DelegationCreatedPayload {
            delegation_id: delegation.id,
            tenant_id: delegation.tenant_id.clone(),
            delegator_id: delegation.delegator_id,
            delegatee_id: delegation.delegatee_id,
            definition_id: delegation.definition_id,
            entity_type: delegation.entity_type.clone(),
            valid_from: delegation.valid_from,
            valid_until: delegation.valid_until,
        };

        let env = envelope::create_envelope(
            event_id,
            delegation.tenant_id.clone(),
            subjects::DELEGATION_CREATED.to_string(),
            payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| DelegationError::Validation(format!("envelope: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::DELEGATION_CREATED,
            "workflow_delegation",
            &delegation.id.to_string(),
            &validated,
        )
        .await?;

        tx.commit().await?;

        Ok(delegation)
    }

    /// Revoke an active delegation.
    /// Guard: delegation must exist and not already be revoked.
    /// Mutation: SET revoked_at, revoked_by, revoke_reason.
    /// Outbox: delegation.revoked event.
    pub async fn revoke(
        pool: &PgPool,
        delegation_id: Uuid,
        req: &RevokeDelegationRequest,
    ) -> Result<DelegationRule, DelegationError> {
        let mut tx = pool.begin().await?;

        // ── Guard: lock row, check tenant, check not revoked ──
        let delegation = sqlx::query_as::<_, DelegationRule>(
            "SELECT * FROM workflow_delegation_rules WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(delegation_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DelegationError::NotFound)?;

        if delegation.revoked_at.is_some() {
            return Err(DelegationError::AlreadyRevoked);
        }

        // ── Mutation ──
        let revoked = sqlx::query_as::<_, DelegationRule>(
            r#"
            UPDATE workflow_delegation_rules
            SET revoked_at = now(),
                revoked_by = $1,
                revoke_reason = $2,
                updated_at = now()
            WHERE id = $3 AND tenant_id = $4
            RETURNING *
            "#,
        )
        .bind(req.revoked_by)
        .bind(&req.revoke_reason)
        .bind(delegation_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox: delegation.revoked ──
        let event_id = Uuid::new_v4();
        let payload = DelegationRevokedPayload {
            delegation_id: revoked.id,
            tenant_id: revoked.tenant_id.clone(),
            delegator_id: revoked.delegator_id,
            delegatee_id: revoked.delegatee_id,
            revoked_by: req.revoked_by,
            revoke_reason: req.revoke_reason.clone(),
        };

        let env = envelope::create_envelope(
            event_id,
            revoked.tenant_id.clone(),
            subjects::DELEGATION_REVOKED.to_string(),
            payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| DelegationError::Validation(format!("envelope: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::DELEGATION_REVOKED,
            "workflow_delegation",
            &revoked.id.to_string(),
            &validated,
        )
        .await?;

        tx.commit().await?;

        Ok(revoked)
    }

    /// Get a delegation by ID.
    pub async fn get(
        pool: &PgPool,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<DelegationRule, DelegationError> {
        sqlx::query_as::<_, DelegationRule>(
            "SELECT * FROM workflow_delegation_rules WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(DelegationError::NotFound)
    }

    /// Resolve the effective actor for a decision.
    /// If the actor has delegated their authority (and the delegation is currently valid),
    /// return the delegatee. Otherwise return None (actor acts as themselves).
    ///
    /// Resolution precedence (most specific wins):
    /// 1. Delegation scoped to (definition_id, entity_type)
    /// 2. Delegation scoped to definition_id only
    /// 3. Delegation scoped to entity_type only
    /// 4. Unscoped delegation (broadest)
    pub async fn resolve_delegatee(
        pool: &PgPool,
        query: &ResolveDelegationQuery,
    ) -> Result<Option<DelegationRule>, DelegationError> {
        // Find all active, non-revoked delegations for this actor
        let delegations = sqlx::query_as::<_, DelegationRule>(
            r#"
            SELECT * FROM workflow_delegation_rules
            WHERE tenant_id = $1
              AND delegator_id = $2
              AND revoked_at IS NULL
              AND valid_from <= now()
              AND (valid_until IS NULL OR valid_until > now())
            ORDER BY created_at DESC
            "#,
        )
        .bind(&query.tenant_id)
        .bind(query.actor_id)
        .fetch_all(pool)
        .await?;

        if delegations.is_empty() {
            return Ok(None);
        }

        // Precedence: most specific match wins
        let mut best: Option<&DelegationRule> = None;
        let mut best_score = 0u8;

        for d in &delegations {
            let def_match = match (d.definition_id, query.definition_id) {
                (Some(a), Some(b)) if a == b => true,
                (None, _) => true,
                _ => false,
            };
            let ent_match = match (&d.entity_type, &query.entity_type) {
                (Some(a), Some(b)) if a == b => true,
                (None, _) => true,
                _ => false,
            };

            if !def_match || !ent_match {
                continue;
            }

            let score = match (d.definition_id.is_some(), d.entity_type.is_some()) {
                (true, true) => 4,
                (true, false) => 3,
                (false, true) => 2,
                (false, false) => 1,
            };

            if score > best_score {
                best_score = score;
                best = Some(d);
            }
        }

        Ok(best.cloned())
    }

    /// List active delegations for a delegator.
    pub async fn list_for_delegator(
        pool: &PgPool,
        tenant_id: &str,
        delegator_id: Uuid,
    ) -> Result<Vec<DelegationRule>, DelegationError> {
        Ok(sqlx::query_as::<_, DelegationRule>(
            r#"
            SELECT * FROM workflow_delegation_rules
            WHERE tenant_id = $1 AND delegator_id = $2 AND revoked_at IS NULL
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(delegator_id)
        .fetch_all(pool)
        .await?)
    }

    /// List active delegations for a delegatee (who has received delegation).
    pub async fn list_for_delegatee(
        pool: &PgPool,
        tenant_id: &str,
        delegatee_id: Uuid,
    ) -> Result<Vec<DelegationRule>, DelegationError> {
        Ok(sqlx::query_as::<_, DelegationRule>(
            r#"
            SELECT * FROM workflow_delegation_rules
            WHERE tenant_id = $1 AND delegatee_id = $2 AND revoked_at IS NULL
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(delegatee_id)
        .fetch_all(pool)
        .await?)
    }
}
