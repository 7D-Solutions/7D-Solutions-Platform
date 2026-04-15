//! Item change history — immutable audit trail for item master governance.
//!
//! Records who changed what, when, and why for every material change to
//! item revisions and policy flags. Each entry includes a structured JSON
//! diff of before/after values.
//!
//! Invariants:
//! - History rows are immutable once written (append-only)
//! - Writes follow Guard → Mutation → Outbox in a single transaction
//! - Idempotent: duplicate idempotency_key returns existing entry
//! - Tenant-scoped: all queries filter by tenant_id

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::{
    build_item_change_recorded_envelope, ItemChangeRecordedPayload, EVENT_TYPE_ITEM_CHANGE_RECORDED,
};

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChangeHistoryEntry {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub change_type: String,
    pub actor_id: String,
    pub diff: serde_json::Value,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordChangeRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub change_type: String,
    pub actor_id: String,
    pub diff: serde_json::Value,
    pub reason: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ChangeHistoryError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotency key conflict: same key used with a different request")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal helpers
// ============================================================================

fn require_non_empty(value: &str, field: &str) -> Result<(), ChangeHistoryError> {
    if value.trim().is_empty() {
        return Err(ChangeHistoryError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

fn validate_request(req: &RecordChangeRequest) -> Result<(), ChangeHistoryError> {
    require_non_empty(&req.tenant_id, "tenant_id")?;
    require_non_empty(&req.change_type, "change_type")?;
    require_non_empty(&req.actor_id, "actor_id")?;
    require_non_empty(&req.idempotency_key, "idempotency_key")?;

    if !matches!(
        req.change_type.as_str(),
        "revision_created" | "revision_activated" | "policy_updated" | "classification_assigned"
    ) {
        return Err(ChangeHistoryError::Validation(format!(
            "change_type must be one of: revision_created, revision_activated, policy_updated, classification_assigned; got '{}'",
            req.change_type
        )));
    }

    Ok(())
}

// ============================================================================
// Record change history (Guard → Mutation → Outbox)
// ============================================================================

/// Record a change history entry within an existing transaction.
///
/// This is designed to be called from within revision operations so the
/// change history is atomic with the business mutation.
///
/// Returns `(ChangeHistoryEntry, is_replay)`.
pub async fn record_change_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &RecordChangeRequest,
) -> Result<(ChangeHistoryEntry, bool), ChangeHistoryError> {
    // --- Guard: validate inputs ---
    validate_request(req)?;

    // --- Guard: idempotency check ---
    let existing: Option<ChangeHistoryEntry> = sqlx::query_as(
        r#"
        SELECT * FROM item_change_history
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(entry) = existing {
        return Ok((entry, true));
    }

    // --- Mutation: insert change history row ---
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let entry = sqlx::query_as::<_, ChangeHistoryEntry>(
        r#"
        INSERT INTO item_change_history
            (tenant_id, item_id, revision_id, change_type, actor_id,
             diff, reason, idempotency_key, created_at)
        VALUES ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, $9)
        RETURNING *
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.revision_id)
    .bind(&req.change_type)
    .bind(&req.actor_id)
    .bind(&req.diff)
    .bind(req.reason.as_deref())
    .bind(&req.idempotency_key)
    .bind(now)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.code().as_deref() == Some("23505") {
                return ChangeHistoryError::ConflictingIdempotencyKey;
            }
        }
        ChangeHistoryError::Database(e)
    })?;

    // --- Outbox: emit audit event ---
    let payload = ItemChangeRecordedPayload {
        change_id: entry.id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_id: req.revision_id,
        change_type: req.change_type.clone(),
        actor_id: req.actor_id.clone(),
        diff: req.diff.clone(),
        reason: req.reason.clone(),
        recorded_at: now,
    };
    let envelope = build_item_change_recorded_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'item_change_history', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_CHANGE_RECORDED)
    .bind(entry.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut **tx)
    .await?;

    Ok((entry, false))
}

/// Record a change history entry as a standalone operation.
///
/// Opens its own transaction. Use `record_change_in_tx` when you need
/// atomicity with another mutation.
///
/// Returns `(ChangeHistoryEntry, is_replay)`.
pub async fn record_change(
    pool: &PgPool,
    req: &RecordChangeRequest,
) -> Result<(ChangeHistoryEntry, bool), ChangeHistoryError> {
    let mut tx = pool.begin().await?;
    let result = record_change_in_tx(&mut tx, req).await?;
    tx.commit().await?;
    Ok(result)
}

// ============================================================================
// Query: list change history
// ============================================================================

/// List all change history entries for a (tenant, item) in chronological order.
pub async fn list_change_history(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<ChangeHistoryEntry>, ChangeHistoryError> {
    let entries = sqlx::query_as::<_, ChangeHistoryEntry>(
        r#"
        SELECT * FROM item_change_history
        WHERE tenant_id = $1 AND item_id = $2
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await?;

    Ok(entries)
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> RecordChangeRequest {
        RecordChangeRequest {
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            revision_id: Some(Uuid::new_v4()),
            change_type: "revision_created".to_string(),
            actor_id: "user-123".to_string(),
            diff: serde_json::json!({"name": {"after": "Widget v2"}}),
            reason: Some("Spec update".to_string()),
            idempotency_key: "idem-1".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn valid_request_passes_validation() {
        assert!(validate_request(&valid_request()).is_ok());
    }

    #[test]
    fn empty_tenant_id_rejected() {
        let mut r = valid_request();
        r.tenant_id = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ChangeHistoryError::Validation(_))
        ));
    }

    #[test]
    fn empty_actor_id_rejected() {
        let mut r = valid_request();
        r.actor_id = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ChangeHistoryError::Validation(_))
        ));
    }

    #[test]
    fn invalid_change_type_rejected() {
        let mut r = valid_request();
        r.change_type = "unknown".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ChangeHistoryError::Validation(_))
        ));
    }

    #[test]
    fn empty_idempotency_key_rejected() {
        let mut r = valid_request();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ChangeHistoryError::Validation(_))
        ));
    }
}
