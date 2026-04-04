//! Hold repository — all SQL for `workflow_holds` and hold idempotency.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{envelope, subjects};
use crate::outbox;

use super::holds::{ApplyHoldRequest, Hold, HoldError, ListHoldsQuery, ReleaseHoldRequest};

// ── Event payloads ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct HoldAppliedPayload {
    hold_id: Uuid,
    tenant_id: String,
    entity_type: String,
    entity_id: String,
    hold_type: String,
    reason: Option<String>,
    applied_by: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct HoldReleasedPayload {
    hold_id: Uuid,
    tenant_id: String,
    entity_type: String,
    entity_id: String,
    hold_type: String,
    released_by: Option<Uuid>,
    release_reason: Option<String>,
}

// ── Idempotency ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct IdempotencyHit {
    response_body: serde_json::Value,
    status_code: i32,
}

async fn check_idempotency(
    pool: &PgPool,
    key: &str,
) -> Result<Option<IdempotencyHit>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyHit>(
        r#"
        SELECT response_body, status_code
        FROM workflow_idempotency_keys
        WHERE app_id = 'hold' AND idempotency_key = $1
          AND expires_at > now()
        "#,
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

async fn store_idempotency(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
    response: &serde_json::Value,
    status_code: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO workflow_idempotency_keys
            (app_id, idempotency_key, response_body, status_code, expires_at)
        VALUES ('hold', $1, $2, $3, now() + interval '24 hours')
        ON CONFLICT (app_id, idempotency_key) DO NOTHING
        "#,
    )
    .bind(key)
    .bind(response)
    .bind(status_code)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Repository ────────────────────────────────────────────────

pub struct HoldRepo;

impl HoldRepo {
    /// Apply a hold to an entity.
    /// Guard: no active hold of the same type on this entity.
    /// Mutation: INSERT hold row.
    /// Outbox: hold.applied event.
    pub async fn apply(pool: &PgPool, req: &ApplyHoldRequest) -> Result<Hold, HoldError> {
        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(hit) = check_idempotency(pool, key).await? {
                let hold: Hold = serde_json::from_value(hit.response_body).map_err(|e| {
                    HoldError::Validation(format!("idempotency replay decode: {}", e))
                })?;
                return Ok(hold);
            }
        }

        // ── Guard: validate required fields ──
        if req.entity_type.is_empty() {
            return Err(HoldError::Validation("entity_type is required".into()));
        }
        if req.entity_id.is_empty() {
            return Err(HoldError::Validation("entity_id is required".into()));
        }
        if req.hold_type.is_empty() {
            return Err(HoldError::Validation("hold_type is required".into()));
        }

        let hold_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let mut tx = pool.begin().await?;

        // ── Guard: check no active hold of same type ──
        let existing = sqlx::query_as::<_, Hold>(
            r#"
            SELECT * FROM workflow_holds
            WHERE tenant_id = $1 AND entity_type = $2 AND entity_id = $3
              AND hold_type = $4 AND released_at IS NULL
            FOR UPDATE
            "#,
        )
        .bind(&req.tenant_id)
        .bind(&req.entity_type)
        .bind(&req.entity_id)
        .bind(&req.hold_type)
        .fetch_optional(&mut *tx)
        .await?;

        if existing.is_some() {
            return Err(HoldError::AlreadyHeld);
        }

        // ── Mutation ──
        let hold = sqlx::query_as::<_, Hold>(
            r#"
            INSERT INTO workflow_holds
                (id, tenant_id, entity_type, entity_id, hold_type,
                 reason, applied_by, metadata, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(hold_id)
        .bind(&req.tenant_id)
        .bind(&req.entity_type)
        .bind(&req.entity_id)
        .bind(&req.hold_type)
        .bind(&req.reason)
        .bind(req.applied_by)
        .bind(&req.metadata)
        .bind(&req.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox event ──
        let payload = HoldAppliedPayload {
            hold_id: hold.id,
            tenant_id: hold.tenant_id.clone(),
            entity_type: hold.entity_type.clone(),
            entity_id: hold.entity_id.clone(),
            hold_type: hold.hold_type.clone(),
            reason: hold.reason.clone(),
            applied_by: hold.applied_by,
        };

        let env = envelope::create_envelope(
            event_id,
            hold.tenant_id.clone(),
            subjects::HOLD_APPLIED.to_string(),
            payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| HoldError::Validation(format!("envelope validation: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::HOLD_APPLIED,
            "workflow_hold",
            &hold.id.to_string(),
            &validated,
        )
        .await?;

        // Store idempotency key
        if let Some(ref key) = req.idempotency_key {
            store_idempotency(
                &mut tx,
                key,
                &serde_json::to_value(&hold).unwrap_or_default(),
                201,
            )
            .await?;
        }

        tx.commit().await?;

        Ok(hold)
    }

    /// Release an active hold.
    /// Guard: hold must exist and not already be released.
    /// Mutation: SET released_at, released_by, release_reason.
    /// Outbox: hold.released event.
    pub async fn release(
        pool: &PgPool,
        hold_id: Uuid,
        req: &ReleaseHoldRequest,
    ) -> Result<Hold, HoldError> {
        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(hit) = check_idempotency(pool, key).await? {
                let hold: Hold = serde_json::from_value(hit.response_body).map_err(|e| {
                    HoldError::Validation(format!("idempotency replay decode: {}", e))
                })?;
                return Ok(hold);
            }
        }

        let mut tx = pool.begin().await?;

        // ── Guard: fetch and lock hold, check tenant, check not released ──
        let hold = sqlx::query_as::<_, Hold>(
            "SELECT * FROM workflow_holds WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(hold_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(HoldError::NotFound)?;

        if hold.released_at.is_some() {
            return Err(HoldError::AlreadyReleased);
        }

        // ── Mutation ──
        let released = sqlx::query_as::<_, Hold>(
            r#"
            UPDATE workflow_holds
            SET released_at = now(),
                released_by = $1,
                release_reason = $2,
                updated_at = now()
            WHERE id = $3 AND tenant_id = $4
            RETURNING *
            "#,
        )
        .bind(req.released_by)
        .bind(&req.release_reason)
        .bind(hold_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox event ──
        let event_id = Uuid::new_v4();
        let payload = HoldReleasedPayload {
            hold_id: released.id,
            tenant_id: released.tenant_id.clone(),
            entity_type: released.entity_type.clone(),
            entity_id: released.entity_id.clone(),
            hold_type: released.hold_type.clone(),
            released_by: released.released_by,
            release_reason: released.release_reason.clone(),
        };

        let env = envelope::create_envelope(
            event_id,
            released.tenant_id.clone(),
            subjects::HOLD_RELEASED.to_string(),
            payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| HoldError::Validation(format!("envelope validation: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::HOLD_RELEASED,
            "workflow_hold",
            &released.id.to_string(),
            &validated,
        )
        .await?;

        // Store idempotency key
        if let Some(ref key) = req.idempotency_key {
            store_idempotency(
                &mut tx,
                key,
                &serde_json::to_value(&released).unwrap_or_default(),
                200,
            )
            .await?;
        }

        tx.commit().await?;

        Ok(released)
    }

    /// Get a single hold by ID (tenant-scoped).
    pub async fn get(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<Hold, HoldError> {
        sqlx::query_as::<_, Hold>(
            "SELECT * FROM workflow_holds WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(HoldError::NotFound)
    }

    /// List holds with optional filters.
    pub async fn list(pool: &PgPool, q: &ListHoldsQuery) -> Result<Vec<Hold>, HoldError> {
        let limit = q.limit.unwrap_or(50).min(200);
        let offset = q.offset.unwrap_or(0);
        let active_only = q.active_only.unwrap_or(false);

        let mut conditions = vec!["tenant_id = $1".to_string()];
        let mut param_idx = 2u32;

        if q.entity_type.is_some() {
            conditions.push(format!("entity_type = ${}", param_idx));
            param_idx += 1;
        }
        if q.entity_id.is_some() {
            conditions.push(format!("entity_id = ${}", param_idx));
            param_idx += 1;
        }
        if q.hold_type.is_some() {
            conditions.push(format!("hold_type = ${}", param_idx));
            param_idx += 1;
        }
        if active_only {
            conditions.push("released_at IS NULL".to_string());
        }

        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            "SELECT * FROM workflow_holds WHERE {} ORDER BY applied_at DESC LIMIT ${} OFFSET ${}",
            where_clause, param_idx, param_idx + 1
        );

        let mut query = sqlx::query_as::<_, Hold>(&query_str).bind(&q.tenant_id);

        if let Some(ref et) = q.entity_type {
            query = query.bind(et);
        }
        if let Some(ref ei) = q.entity_id {
            query = query.bind(ei);
        }
        if let Some(ref ht) = q.hold_type {
            query = query.bind(ht);
        }

        query = query.bind(limit).bind(offset);

        Ok(query.fetch_all(pool).await?)
    }
}
